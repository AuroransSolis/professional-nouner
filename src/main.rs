use chrono::{DateTime, TimeDelta, Timelike, Utc};
use fuzzy_matcher::FuzzyMatcher;
use getrandom::SysRng;
use poise::CreateReply;
use rand::{RngExt, rand_core::UnwrapErr};
use serde::{Deserialize, Serialize};
use serenity::{
    Client,
    all::{ChannelId, GatewayIntents, GuildId, UserId, prelude::EventHandler},
    async_trait,
    builder::{CreateEmbed, CreateEmbedAuthor, CreateMessage},
    model::user::OnlineStatus,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::RwLock};

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Config {
    servers: Servers,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Servers(HashMap<GuildId, ServerConfig>);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ServerConfig {
    update_channel: ChannelId,
    #[serde(flatten)]
    users: HashMap<UserId, UserData>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UserData {
    current: usize,
    last_set: DateTime<Utc>,
    pronouns: Vec<String>,
}

#[derive(clap::Parser)]
struct Options {
    #[arg(short, long, default_value_t = 1)]
    jobs: usize,
}

type Data = Arc<RwLock<HashMap<GuildId, RwLock<ServerConfig>>>>;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[tokio::main]
async fn main() {
    let token = env!("DISCORD_TOKEN", "`DISCORD_TOKEN` envvar not set");
    let data_string =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data.toml")).unwrap();
    let data = toml::from_str::<Config>(&data_string).unwrap();
    let mut bot_data = HashMap::with_capacity(data.servers.0.len());
    data.servers
        .0
        .into_iter()
        .for_each(|(k, v)| assert!(bot_data.insert(k, RwLock::new(v)).is_none()));
    let bot_data = Arc::new(RwLock::new(bot_data));
    let init_data = bot_data.clone();

    let intents = GatewayIntents::GUILD_MEMBERS | GatewayIntents::DIRECT_MESSAGES | GatewayIntents::GUILDS;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                commands(),
                deregister(),
                register(),
                registrar(),
                reroll(),
                set_announce_channel(),
            ],
            on_error: |err| {
                Box::pin(async move {
                    let Some(ctx) = err.ctx() else {
                        println!("Error with no context:\n{err:?}");
                        return;
                    };
                    match err {
                        poise::FrameworkError::CooldownHit {
                            remaining_cooldown,
                            ctx,
                            ..
                        } => {
                            let _ = ctx
                                .say(format!(
                                    "Oops, I've hit a cooldown! Try again in {} seconds.",
                                    remaining_cooldown.as_millis().div_ceil(1000),
                                ))
                                .await;
                        }
                        ref other => {
                            other.ctx().map(async |ctx| {
                                let _ = ctx
                                    .say(format!("Encountered unexpected error: {other:?}"))
                                    .await;
                            });
                        }
                    }
                    println!(
                        "Command {:?} triggered by {} produced error: {err:?}",
                        ctx.command().qualified_name,
                        ctx.author().name
                    );
                })
            },
            ..Default::default()
        })
        .setup(|ctx, _, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                // if let Err(err) = poise::builtins::register_globally(ctx, &[deregister()]).await {
                //     println!("Error in setup: failed to register global commands!\n{err}");
                //     return Err(err);
                // }
                Ok(init_data)
            })
        })
        .build();

    let mut client = Client::builder(token, intents)
        .framework(framework)
        .event_handler(Handler)
        .await
        .expect("Failed to start client!");

    let die0 = Arc::new(AtomicBool::new(false));
    let die1 = die0.clone();

    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Could not register Ctrl+C handler");
        die0.store(true, Ordering::Relaxed);
        let _ = shard_manager.runners.try_lock().map(|runners| {
            runners
                .iter()
                .for_each(|(_, v)| v.runner_tx.set_status(OnlineStatus::Offline))
        });
        shard_manager.shutdown_all().await;
    });

    let http = client.http.clone();
    let cache = client.cache.clone();
    tokio::spawn(async move {
        while !die1.load(Ordering::Relaxed) {
            let until_midnight = Utc::now()
                .with_hour(20)
                .unwrap()
                .with_minute(52)
                .unwrap()
                .with_second(0)
                .unwrap()
                .signed_duration_since(Utc::now())
                .to_std()
                .unwrap();
            println!("Sleeping for: {:?}", until_midnight);
            tokio::time::sleep(until_midnight).await;
            let now = Utc::now();
            println!("Running daily update");
            for (guild_id, guild_cfg) in bot_data.read().await.iter() {
                let channel_id = guild_cfg.read().await.update_channel;
                if guild_cfg.read().await.users.is_empty() {
                    continue;
                }
                let user_data = guild_cfg
                    .write()
                    .await
                    .users
                    .iter_mut()
                    .map(|(user_id, user_data)| {
                        user_data.last_set = now;
                        user_data.current =
                            UnwrapErr(SysRng).random_range(0..user_data.pronouns.len());
                        (*user_id, user_data.pronouns[user_data.current].clone())
                    })
                    .collect::<Vec<_>>();
                let mut fields = Vec::with_capacity(user_data.len());
                for (id, current_pronoun) in user_data.into_iter() {
                    let name = match http.get_member(*guild_id, id).await {
                        Ok(member) => member.display_name().to_string(),
                        _ => match http.get_user(id).await {
                            Ok(user) => user.name,
                            _ => format!("ID: {id}"),
                        },
                    };
                    fields.push((name, current_pronoun, false));
                }
                let msg = CreateMessage::new().embed(
                    CreateEmbed::new()
                        .title("The Pronoun Update")
                        .description(
                            "Here's the new pronouns for the registered users in this server!",
                        )
                        .fields(fields),
                );
                if let Err(err) = channel_id.send_message((&cache, http.as_ref()), msg).await {
                    println!("Failed to send registrar update to `{guild_id}/{channel_id}`: {err:?}");
                } else {
                    println!("Posted registrar update to guild id `{guild_id}/{channel_id}` successfully");
                }
            }
            if let Err(err) = write_cfg_file_noreply(&bot_data).await {
                println!("Error writing updated data: {err:?}");
            }
            tokio::time::sleep(std::time::Duration::from_hours(1)).await;
        }
    });

    if let Err(err) = client.start_autosharded().await {
        println!("Error starting the client: {err}");
    }
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(
        &self,
        ctx: poise::serenity_prelude::Context,
        _: poise::serenity_prelude::Ready,
    ) {
        ctx.online();
    }
}

#[poise::command(slash_command)]
pub async fn commands(ctx: Context<'_>) -> Result<(), Error> {
    poise::builtins::register_application_commands_buttons(ctx).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    install_context = "Guild",
    interaction_context = "Guild|BotDm"
)]
pub async fn deregister(ctx: Context<'_>) -> Result<(), Error> {
    let user_id = ctx.author().id;
    let modified = match ctx.guild_id() {
        Some(guild_id) => {
            if let Some(server_cfg) = ctx.data().read().await.get(&guild_id) {
                server_cfg.write().await.users.remove(&user_id).is_some()
            } else {
                false
            }
        }
        None => {
            let mut modified = false;
            for server_cfg in ctx.data().read().await.values() {
                modified |= server_cfg.write().await.users.remove(&user_id).is_some();
            }
            modified
        }
    };
    if modified {
        write_cfg_file(ctx).await?;
        let reply = if ctx.guild_id().is_some() {
            "Deregistered you in this guild!"
        } else {
            "Deregistered you from all guilds!"
        };
        ctx.send(CreateReply::default().content(reply).ephemeral(true))
            .await?;
    } else {
        ctx.send(
            CreateReply::default()
                .content("No changes made - are you registered?")
                .ephemeral(true),
        )
        .await?;
    }
    Ok(())
}

#[poise::command(
    slash_command,
    subcommands("register_copy", "register_new"),
    subcommand_required,
    install_context = "Guild",
    interaction_context = "Guild"
)]
pub async fn register(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "copy",
    // install_context = "Guild",
    // interaction_context = "Guild"
)]
pub async fn register_copy(ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "new",
    // install_context = "Guild",
    // interaction_context = "Guild"
)]
pub async fn register_new(ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(
    slash_command,
    install_context = "Guild",
    interaction_context = "Guild"
)]
pub async fn registrar(ctx: Context<'_>) -> Result<(), Error> {
    // let now = Utc::now();
    let Some(partial_guild) = ctx.partial_guild().await else {
        ctx.send(
            CreateReply::default()
                .content("Internal error! Try again.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    };
    let user_data = ctx
        .data()
        .read()
        .await
        .get(&ctx.guild_id().unwrap())
        .unwrap()
        .write()
        .await
        .users
        .iter()
        // .iter_mut()
        .map(|(user_id, user_data)| {
            // if now.signed_duration_since(user_data.last_set).abs() >= TimeDelta::days(1) {
            // let new_idx = UnwrapErr(SysRng).random_range(0..user_data.pronouns.len());
            // user_data.current = new_idx;
            // user_data.last_set = now;
            // changed = true;
            // }
            (*user_id, user_data.pronouns[user_data.current].clone())
        })
        .collect::<Vec<_>>();
    let mut fields = Vec::with_capacity(user_data.len());
    for (id, current_pronoun) in user_data.into_iter() {
        let member = partial_guild.member(ctx.http(), id).await?;
        fields.push((member.display_name().to_string(), current_pronoun, false))
    }
    fields.sort_by(|a, b| a.0.cmp(&b.0));

    // let my_name = ctx.cache().current_user().name.clone();
    // let my_icon_url = ctx.cache().current_user().avatar_url().unwrap();

    ctx.send(
        CreateReply::default().embed(
            CreateEmbed::new()
                .title("Registrar")
                // .author(CreateEmbedAuthor::new(my_name).icon_url(my_icon_url))
                .fields(fields),
        ),
    )
    .await?;

    Ok(())
}

#[poise::command(
    slash_command,
    install_context = "Guild|User",
    interaction_context = "Guild|BotDm"
)]
pub async fn reroll(ctx: Context<'_>) -> Result<(), Error> {
    let modified = if let Some(guild_id) = ctx.guild_id() {
        if let Some(user_data) = ctx
            .data()
            .read()
            .await
            .get(&guild_id)
            .unwrap()
            .write()
            .await
            .users
            .get_mut(&ctx.author().id)
        {
            user_data.last_set = Utc::now();
            user_data.current = UnwrapErr(SysRng).random_range(0..user_data.pronouns.len());
            ctx.reply(format!(
                "Your new pronouns are: `{}`!",
                user_data.pronouns[user_data.current]
            ))
            .await?;
            true
        } else {
            ctx.send(
                CreateReply::default()
                    .content("You're not registered in this server!")
                    .ephemeral(true),
            )
            .await?;
            false
        }
    } else {
        let now = Utc::now();
        let mut unregistered = true;
        for server in ctx.data().read().await.values() {
            if let Some(user_data) = server.write().await.users.get_mut(&ctx.author().id) {
                user_data.last_set = now;
                user_data.current = UnwrapErr(SysRng).random_range(0..user_data.pronouns.len());
                unregistered = false;
            }
        }
        if unregistered {
            ctx.reply("You're not registered in any servers!").await?;
            false
        } else {
            ctx.reply("Rerolled your pronouns in all servers you're registered in.")
                .await?;
            true
        }
    };
    if modified {
        write_cfg_file(ctx).await?;
    }
    Ok(())
}

async fn autocomplete_channels(ctx: Context<'_>, partial: &str) -> impl Iterator<Item = String> {
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
    ctx.partial_guild()
        .await
        .unwrap()
        .channels(ctx.http())
        .await
        .unwrap()
        .into_values()
        .filter(move |channel| matcher.fuzzy_match(channel.name(), partial).is_some())
        .map(|channel| channel.name)
}

#[poise::command(
    slash_command,
    install_context = "Guild",
    interaction_context = "Guild",
    rename = "announcement",
    required_bot_permissions = "SEND_MESSAGES"
)]
pub async fn set_announce_channel(
    ctx: Context<'_>,
    #[description = "Set the channel to send the daily update in"]
    #[autocomplete = "autocomplete_channels"]
    channel: Option<poise::serenity_prelude::GuildChannel>,
) -> Result<(), Error> {
    let guild_channel = match channel {
        Some(channel) => channel,
        None => ctx.guild_channel().await.unwrap()
    };
    let my_perms_in_channel = {
        let guild = ctx.guild().unwrap();
        let my_member = guild.members.get(&ctx.framework().bot_id).unwrap();
        guild.user_permissions_in(&guild_channel, my_member)
        // let pg = ctx.partial_guild().await.unwrap();
        // let my_member = pg.member(ctx, ctx.framework().bot_id).await.unwrap();
        // pg.user_permissions_in(&guild_channel, &my_member)
    };
    if my_perms_in_channel.send_messages() {
        let channel_id = guild_channel.id;
        println!(
            "Seems to be possible to send messages in {}/{}. Updating context and writing data.",
            ctx.guild_id().unwrap(),
            channel_id,
        );
        ctx.data()
            .read()
            .await
            .get(&ctx.guild_id().unwrap())
            .unwrap()
            .write()
            .await
            .update_channel = channel_id;
        ctx.reply(format!(
            "Successfully set `{}` to be the announcement channel for guild `{}`.",
            channel_id,
            ctx.guild_id().unwrap(),
        ))
        .await?;
        write_cfg_file(ctx).await?;
        Ok(())
    } else {
        ctx.reply("I can't send messages in that channel!").await?;
        Err("I can't send messages in that channel!".to_string().into())
    }
}

async fn write_cfg_file(ctx: Context<'_>) -> Result<(), Error> {
    let new_cfg = {
        let read = ctx.data().read().await;
        let mut new_cfg = Config {
            servers: Servers(HashMap::with_capacity(read.len())),
        };
        for (gid, scfg) in read.iter() {
            let _ = new_cfg.servers.0.insert(*gid, scfg.read().await.clone());
        }
        new_cfg
    };
    let formatted = toml::to_string_pretty(&new_cfg).unwrap();
    let mut file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(concat!(env!("CARGO_MANIFEST_DIR"), "/data.toml"))
        .await
    {
        Ok(file) => file,
        Err(err) => {
            println!("Error opening data file for writing: {err}");
            let _ = ctx.reply("Failed to save changes! Please retry.").await;
            return Err(Box::new(err));
        }
    };
    if let Err(err) = file.write_all(formatted.as_bytes()).await {
        println!("Error writing updated data to file: {err}");
        let _ = ctx.reply("Failed to save changes! Please retry.").await;
        return Err(Box::new(err));
    }
    println!("Wrote new config to file.");
    Ok(())
}

async fn write_cfg_file_noreply(data: &Data) -> Result<(), Error> {
    let new_cfg = {
        let read = data.read().await;
        let mut new_cfg = Config {
            servers: Servers(HashMap::with_capacity(read.len())),
        };
        for (gid, scfg) in read.iter() {
            let _ = new_cfg.servers.0.insert(*gid, scfg.read().await.clone());
        }
        new_cfg
    };
    let formatted = toml::to_string_pretty(&new_cfg).unwrap();
    let mut file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(concat!(env!("CARGO_MANIFEST_DIR"), "/data.toml"))
        .await
    {
        Ok(file) => file,
        Err(err) => {
            println!("Error opening data file for writing: {err}");
            return Err(Box::new(err));
        }
    };
    if let Err(err) = file.write_all(formatted.as_bytes()).await {
        println!("Error writing updated data to file: {err}");
        return Err(Box::new(err));
    }
    println!("Wrote new config to file.");
    Ok(())
}
