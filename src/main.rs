#![feature(iter_intersperse)]

use chrono::{DateTime, Timelike, Utc};
use fuzzy_matcher::FuzzyMatcher;
use getrandom::SysRng;
use poise::CreateReply;
use rand::{RngExt, rand_core::UnwrapErr};
use serde::{Deserialize, Serialize};
use serenity::{
    Client,
    all::{ChannelId, GatewayIntents, GuildId, UserId, prelude::EventHandler},
    async_trait,
    builder::{CreateEmbed, CreateMessage, EditMember},
    model::{guild::Guild, user::OnlineStatus},
};
use std::{
    collections::HashMap,
    fmt::Write,
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

#[derive(Debug)]
struct ServerConfigCtx {
    update_channel: ChannelId,
    users: HashMap<UserId, RwLock<UserData>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UserData {
    current: usize,
    last_set: DateTime<Utc>,
    change_nickname: bool,
    pronouns: Vec<String>,
}

#[derive(clap::Parser)]
struct Options {
    #[arg(short, long, default_value_t = 1)]
    jobs: usize,
}

type Data = Arc<RwLock<HashMap<GuildId, RwLock<ServerConfigCtx>>>>;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[tokio::main]
async fn main() {
    let token = env!("DISCORD_TOKEN", "`DISCORD_TOKEN` envvar not set");
    let data_string =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data.toml")).unwrap();
    let data = toml::from_str::<Config>(&data_string).unwrap();
    let mut bot_data = HashMap::with_capacity(data.servers.0.len());
    data.servers.0.into_iter().for_each(|(k, v)| {
        let mut users = HashMap::with_capacity(v.users.len());
        v.users
            .into_iter()
            .for_each(|(k, v)| assert!(users.insert(k, RwLock::new(v)).is_none()));
        assert!(
            bot_data
                .insert(
                    k,
                    RwLock::new(ServerConfigCtx {
                        update_channel: v.update_channel,
                        users
                    })
                )
                .is_none()
        );
    });
    let bot_data = Arc::new(RwLock::new(bot_data));
    let init_data = bot_data.clone();

    let intents = GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::GUILDS
        | GatewayIntents::GUILD_PRESENCES;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                commands(),
                deregister(),
                register(),
                registrar(),
                reroll(),
                set_announce_channel(),
                settings(),
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
                .with_hour(23)
                .unwrap()
                .with_minute(59)
                .unwrap()
                .with_second(59)
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
                let mut user_data = Vec::new();
                for (user_id, ud) in guild_cfg.read().await.users.iter() {
                    let mut ud_lock = ud.write().await;
                    ud_lock.last_set = now;
                    let prev = ud_lock.pronouns[ud_lock.current].clone();
                    ud_lock.current =
                        UnwrapErr(SysRng).random_range(0..ud_lock.pronouns.len());
                    user_data.push((
                        *user_id,
                        prev,
                        ud_lock.pronouns[ud_lock.current].clone(),
                        ud_lock.change_nickname,
                    ));
                }
                let mut fields = Vec::with_capacity(user_data.len());
                let mut too_long = Vec::new();
                let mut permissions = Vec::new();
                for (id, prev_pronoun, current_pronoun, change_nickname) in user_data.into_iter() {
                    let name = match http.get_member(*guild_id, id).await {
                        Ok(mut member) => {
                            let name = member.display_name().to_string();
                            let prev_suffix = format!(" | {prev_pronoun}");
                            if change_nickname {
                                let cleaned =
                                    name.strip_suffix(&prev_suffix).unwrap_or(name.as_str());
                                if name.len() + current_pronoun.len() + 3 <= 32 {
                                    let name = format!("{cleaned} | {current_pronoun}");
                                    if member
                                        .edit(
                                            (&cache, http.as_ref()),
                                            EditMember::new().nickname(name),
                                        )
                                        .await
                                        .is_err()
                                    {
                                        permissions.push(cleaned.to_string());
                                    };
                                } else {
                                    too_long.push(name.to_string());
                                }
                            }
                            name
                        }
                        _ => match http.get_user(id).await {
                            Ok(user) => user.name,
                            _ => format!("ID: {id}"),
                        },
                    };
                    fields.push((name, current_pronoun, false));
                }
                if !too_long.is_empty() {
                    fields.push((
                        concat!(
                            "The following users requested nickname changes, but their nicknames are ",
                            "too long:",
                        )
                        .to_string(),
                        too_long.into_iter().intersperse(", ".to_string()).collect(),
                        false,
                    ));
                }
                if !permissions.is_empty() {
                    fields.push((
                        concat!(
                            "Failed to set nicknames for these users due to permissions errors (is ",
                            "my role higher than all these users'?):"
                        )
                        .to_string(),
                        permissions
                            .into_iter()
                            .intersperse(", ".to_string())
                            .collect(),
                        false,
                    ));
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
                    println!(
                        "Failed to send registrar update to `{guild_id}/{channel_id}`: {err:?}"
                    );
                } else {
                    println!(
                        "Posted registrar update to guild id `{guild_id}/{channel_id}` successfully"
                    );
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
/// Sends command registration buttons
async fn commands(ctx: Context<'_>) -> Result<(), Error> {
    poise::builtins::register_application_commands_buttons(ctx).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    install_context = "Guild",
    interaction_context = "Guild|BotDm"
)]
/// Remove your user registration. In DMs removes all registrations, in server removes that server only.
async fn deregister(ctx: Context<'_>) -> Result<(), Error> {
    let user_id = ctx.author().id;
    println!("Deregistering user `{user_id}`");
    let modified = match ctx.guild_id() {
        Some(guild_id) => {
            if let Some(user_data) = ctx
                .data()
                .read()
                .await
                .get(&ctx.guild_id().unwrap())
                .unwrap()
                .write()
                .await
                .users
                .remove(&user_id)
            {
                let user_data = user_data.read().await;
                if user_data.change_nickname {
                    let suffix = format!(" | {}", user_data.pronouns[user_data.current]);
                    let mut member = guild_id.member(ctx, user_id).await.unwrap();
                    if let Some(cleaned) = member.display_name().strip_suffix(&suffix) {
                        member
                            .edit(ctx, EditMember::new().nickname(cleaned))
                            .await
                            .unwrap();
                    }
                }
                true
            } else {
                false
            }
        }
        None => {
            let mut modified = false;
            for (gid, server_cfg) in ctx.data().read().await.iter() {
                modified |= if let Some(user_data) = server_cfg.write().await.users.remove(&user_id)
                {
                    let user_data = user_data.read().await;
                    if user_data.change_nickname {
                        let suffix = format!(" | {}", user_data.pronouns[user_data.current]);
                        let mut member = gid.member(ctx, user_id).await.unwrap();
                        if let Some(cleaned) = member.display_name().strip_suffix(&suffix) {
                            member
                                .edit(ctx, EditMember::new().nickname(cleaned))
                                .await
                                .unwrap();
                        }
                    }
                    true
                } else {
                    false
                };
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
async fn register(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

async fn autocomplete_registered_guilds(
    ctx: Context<'_>,
    partial: &str,
) -> impl Iterator<Item = String> {
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
    let mut collection = Vec::new();
    for (gid, cfg) in ctx.data().read().await.iter() {
        if cfg.read().await.users.contains_key(&ctx.author().id) {
            let guild_name = ctx.http().get_guild(*gid).await.unwrap().name;
            if matcher.fuzzy_match(&guild_name, partial).is_some() {
                collection.push(guild_name);
            }
        }
    }
    collection.into_iter()
}

async fn user_is_registered(ctx: Context<'_>) -> Result<bool, Error> {
    if let Some(cfg) = ctx.data().read().await.get(&ctx.guild_id().unwrap()) {
        Ok(cfg.read().await.users.contains_key(&ctx.author().id))
    } else {
        Ok(false)
    }
}

async fn user_not_registered(ctx: Context<'_>) -> Result<bool, Error> {
    user_is_registered(ctx).await.map(std::ops::Not::not)
}

#[poise::command(
    slash_command,
    rename = "copy",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "user_not_registered",
    ephemeral = true
)]
/// Copy your pronouns from another server to the one you send this command in
async fn register_copy(
    ctx: Context<'_>,
    #[description = "Server to copy from"]
    #[autocomplete = "autocomplete_registered_guilds"]
    guild: Guild,
) -> Result<(), Error> {
    println!(
        "Registering user `{}` by copying from guild `{}`",
        ctx.author().id,
        guild.id
    );
    let mut copy = ctx
        .data()
        .read()
        .await
        .get(&guild.id)
        .unwrap()
        .read()
        .await
        .users
        .get(&ctx.author().id)
        .unwrap()
        .read()
        .await
        .clone();
    copy.last_set = Utc::now();
    let _ = ctx
        .data()
        .read()
        .await
        .get(&ctx.guild_id().unwrap())
        .unwrap()
        .write()
        .await
        .users
        .insert(ctx.author().id, RwLock::new(copy));
    write_cfg_file(ctx).await?;
    ctx.reply("Registration successful!").await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "new",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "user_not_registered",
    ephemeral = true
)]
/// Register with this server, and provide a list of pronouns to use
async fn register_new(
    ctx: Context<'_>,
    #[description = "Should I try and append your pronouns to your nickname when they're updated?"]
    change_nickname: Option<bool>,
    #[description = "Pronouns to register with. Separate with commas, must be alphabetic and under 10 chars."]
    pronouns: String,
) -> Result<(), Error> {
    println!("Registering user `{}` with new data", ctx.author().id);
    let mut change_nickname = change_nickname.unwrap_or(false);
    let mut reply = String::new();
    if ctx.guild().unwrap().owner_id == ctx.author().id && change_nickname {
        writeln!(
            reply,
            "You requested that I modify your nickname, but I can't modify the owner's nickname.",
        )
        .unwrap();
        change_nickname = false;
    }
    let pronouns = pronouns.split(',').map(str::to_string).collect::<Vec<_>>();
    let mut longest = (0, String::new());
    let mut errored = true;
    for pn in &pronouns {
        if !pn.chars().all(|c| c.is_alphabetic() || c == '/') {
            writeln!(
                reply,
                "Pronoun `{pn}` contains non-alphabetic character that is not `/`!"
            )
            .unwrap();
            errored = true;
        }
        if pn.len() > 10 {
            writeln!(reply, "Pronoun `{pn}` has length exceeding maximum (10)!").unwrap();
            errored = true;
        }
        if longest.0 < pn.len() {
            longest = (pn.len(), pn.clone());
        }
    }
    if errored {
        ctx.reply(&reply).await?;
        return Err(reply.into());
    }
    let user_data = UserData {
        current: UnwrapErr(SysRng).random_range(0..pronouns.len()),
        last_set: Utc::now(),
        change_nickname,
        pronouns,
    };
    let _ = ctx
        .data()
        .read()
        .await
        .get(&ctx.guild_id().unwrap())
        .unwrap()
        .write()
        .await
        .users
        .insert(ctx.author().id, RwLock::new(user_data));
    write_cfg_file(ctx).await?;
    let cur_name = ctx
        .partial_guild()
        .await
        .unwrap()
        .member(ctx, ctx.author().id)
        .await
        .unwrap()
        .display_name()
        .to_string();
    if change_nickname && cur_name.len() + 3 + longest.0 > 32 {
        writeln!(
            reply,
            concat!(
                "Registration successful! Warning: you asked me to change your nickname, but the ",
                "longest pronouns in your list (`{}`) combined with your nickname (`{}`) is {} ",
                "characters over the limit. I will not attempt to modify your nickname until this ",
                "is resolved.",
            ),
            longest.1,
            cur_name,
            cur_name.len() + 3 + longest.0 - 32,
        )
        .unwrap();
        ctx.reply(reply).await?;
    } else {
        ctx.reply("Registration successful!").await?;
    }
    Ok(())
}

#[poise::command(
    slash_command,
    install_context = "Guild",
    interaction_context = "Guild"
)]
/// Shows the current listing of pronouns for registered users in this server
async fn registrar(ctx: Context<'_>) -> Result<(), Error> {
    println!(
        "Accessing the registrar for server `{}`",
        ctx.guild_id().unwrap()
    );
    let Some(partial_guild) = ctx.partial_guild().await else {
        ctx.send(
            CreateReply::default()
                .content("Internal error! Try again.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    };
    let mut user_data = Vec::new();
    for (user_id, ud) in ctx.data().read().await.get(&ctx.guild_id().unwrap()).unwrap().read().await.users.iter() {
        let ud_lock = ud.read().await;
        user_data.push((*user_id, ud_lock.pronouns[ud_lock.current].clone()))
    }
    let mut fields = Vec::with_capacity(user_data.len());
    for (id, current_pronoun) in user_data.into_iter() {
        let member = partial_guild.member(ctx, id).await?;
        let name = member
            .display_name()
            .strip_suffix(&format!(" | {current_pronoun}"))
            .unwrap_or(&member.display_name());
        fields.push((name.to_string(), current_pronoun, false))
    }
    fields.sort_by(|a, b| a.0.cmp(&b.0));

    ctx.send(CreateReply::default().embed(CreateEmbed::new().title("Registrar").fields(fields)))
        .await?;

    Ok(())
}

#[poise::command(
    slash_command,
    install_context = "Guild|User",
    interaction_context = "Guild|BotDm"
)]
/// Reroll your pronouns in this server
async fn reroll(ctx: Context<'_>) -> Result<(), Error> {
    println!(
        "Rerolling pronouns for user `{}` in server `{}`",
        ctx.author().id,
        ctx.guild_id().unwrap()
    );
    let modified = if let Some(guild_id) = ctx.guild_id() {
        let (old, change_username) = if let Some(user_data) = ctx
            .data()
            .read()
            .await
            .get(&guild_id)
            .unwrap()
            .read()
            .await
            .users
            .get(&ctx.author().id)
        {
            let mut user_data = user_data.write().await;
            let old = user_data.pronouns[user_data.current].clone();
            user_data.last_set = Utc::now();
            user_data.current = UnwrapErr(SysRng).random_range(0..user_data.pronouns.len());
            ctx.reply(format!(
                "Your new pronouns are: `{}`!",
                user_data.pronouns[user_data.current]
            ))
            .await?;
            (
                Some((old, user_data.pronouns[user_data.current].clone())),
                user_data.change_nickname,
            )
        } else {
            ctx.send(
                CreateReply::default()
                    .content("You're not registered in this server!")
                    .ephemeral(true),
            )
            .await?;
            (None, false)
        };
        if let Some((old_pn, new_pn)) = old.as_ref()
            && change_username
        {
            let mut member = guild_id.member(ctx, ctx.author().id).await.unwrap();
            let prev = member.display_name().to_string();
            let mut new = prev
                .strip_suffix(&format!(" | {old_pn}"))
                .map(|s| s.to_string())
                .unwrap_or(prev);
            new.push_str(" | ");
            new.push_str(&new_pn);
            member
                .edit(ctx, EditMember::new().nickname(new))
                .await
                .unwrap();
            true
        } else {
            old.is_some()
        }
    } else {
        let now = Utc::now();
        let mut unregistered = true;
        for (gid, server) in ctx.data().read().await.iter() {
            let (old_pn, new_pn, change) =
                if let Some(user_data) = server.read().await.users.get(&ctx.author().id) {
                    let mut user_data = user_data.write().await;
                    user_data.last_set = now;
                    let old = user_data.pronouns[user_data.current].clone();
                    user_data.current = UnwrapErr(SysRng).random_range(0..user_data.pronouns.len());
                    unregistered = false;
                    (
                        old,
                        user_data.pronouns[user_data.current].clone(),
                        user_data.change_nickname,
                    )
                } else {
                    continue;
                };
            if change {
                let mut member = gid.member(ctx, ctx.author().id).await.unwrap();
                let prev = member.display_name().to_string();
                let mut new = prev
                    .strip_suffix(&format!(" | {old_pn}"))
                    .map(|s| s.to_string())
                    .unwrap_or(prev);
                new.push_str(" | ");
                new.push_str(&new_pn);
                member
                    .edit(ctx, EditMember::new().nickname(new))
                    .await
                    .unwrap();
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
    rename = "announcement"
)]
/// Set the channel to send the pronouns update in for this server
async fn set_announce_channel(
    ctx: Context<'_>,
    #[description = "Set the channel to send the daily update in"]
    #[autocomplete = "autocomplete_channels"]
    channel: Option<poise::serenity_prelude::GuildChannel>,
) -> Result<(), Error> {
    let guild_channel = match channel {
        Some(channel) => channel,
        None => ctx.guild_channel().await.unwrap(),
    };
    println!(
        "Setting announcement channel for server `{}` to `{}`",
        ctx.guild_id().unwrap(),
        guild_channel.id
    );
    let my_perms_in_channel = {
        let guild = ctx.guild().unwrap();
        let my_member = guild.members.get(&ctx.framework().bot_id).unwrap();
        guild.user_permissions_in(&guild_channel, my_member)
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

#[poise::command(
    slash_command,
    subcommands("settings_cn", "settings_pn"),
    subcommand_required,
    install_context = "Guild",
    interaction_context = "Guild"
)]
async fn settings(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "nickname",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "user_is_registered",
    ephemeral = true
)]
/// Get/set whether the bot should attempt to change your nickname on registrar updates
async fn settings_cn(
    ctx: Context<'_>,
    #[description = "Allow this bot to attempt to change your nickname on registrar updates."]
    change_nickname: Option<bool>,
) -> Result<(), Error> {
    match change_nickname {
        Some(set) => settings_cn_set(ctx, set).await,
        None => settings_cn_get(ctx).await,
    }
}

async fn settings_cn_get(ctx: Context<'_>) -> Result<(), Error> {
    println!(
        "Fetching `change_nickname` for user `{}` in server `{}`",
        ctx.author().id,
        ctx.guild_id().unwrap()
    );
    let current = ctx
        .data()
        .read()
        .await
        .get(&ctx.guild_id().unwrap())
        .unwrap()
        .read()
        .await
        .users
        .get(&ctx.author().id)
        .unwrap()
        .read()
        .await
        .change_nickname;
    if current {
        ctx.reply("You have nickname changing enabled.").await?;
    } else {
        ctx.reply("You have nickname changing disabled.").await?;
    }
    Ok(())
}

async fn settings_cn_set(ctx: Context<'_>, change_nickname: bool) -> Result<(), Error> {
    println!(
        "Changing `change_nickname` for user `{}` in server `{}`",
        ctx.author().id,
        ctx.guild_id().unwrap()
    );
    if ctx.guild().unwrap().owner_id == ctx.author().id {
        ctx.reply("I can't change your nickname, you're the server owner!")
            .await?;
        return Ok(());
    }
    ctx.data()
        .read()
        .await
        .get(&ctx.guild_id().unwrap())
        .unwrap()
        .read()
        .await
        .users
        .get(&ctx.author().id)
        .unwrap()
        .write()
        .await
        .change_nickname = change_nickname;
    write_cfg_file(ctx).await?;
    if change_nickname {
        ctx.reply("Enabled nickname changing.").await?;
    } else {
        ctx.reply("Disabled nickname changing.").await?;
    }
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "pronouns",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "user_is_registered",
    ephemeral = true
)]
/// Get/set your pronouns
async fn settings_pn(
    ctx: Context<'_>,
    #[description = "Separate groups with commas, each must be alphabetic and under 10 chars."]
    pronouns: Option<String>,
) -> Result<(), Error> {
    match pronouns {
        Some(set) => settings_pn_set(ctx, set).await,
        None => settings_pn_get(ctx).await,
    }
}

async fn settings_pn_get(ctx: Context<'_>) -> Result<(), Error> {
    println!(
        "Fetching pronouns for user `{}` in server `{}`",
        ctx.author().id,
        ctx.guild_id().unwrap()
    );
    let current = ctx
        .data()
        .read()
        .await
        .get(&ctx.guild_id().unwrap())
        .unwrap()
        .read()
        .await
        .users
        .get(&ctx.author().id)
        .unwrap()
        .read()
        .await
        .pronouns
        .iter()
        .map(String::as_str)
        .intersperse(",")
        .collect::<String>();
    ctx.reply(format!("Your current pronouns are: `{current}`."))
        .await?;
    Ok(())
}

async fn settings_pn_set(ctx: Context<'_>, pronouns: String) -> Result<(), Error> {
    println!(
        "Changing pronouns for user `{}` in server `{}`",
        ctx.author().id,
        ctx.guild_id().unwrap()
    );
    let pronouns = pronouns.split(',').map(str::to_string).collect::<Vec<_>>();
    let mut longest = (0, String::new());
    for pn in &pronouns {
        if !pn.chars().all(|c| c.is_alphabetic() || c == '/') {
            let msg = format!("Pronoun `{pn}` contains non-alphabetic character that is not `/`!");
            ctx.reply(&msg).await?;
            return Err(msg.into());
        }
        if pn.len() > 10 {
            let msg = format!("Pronoun `{pn}` has length exceeding maximum (10)!");
            ctx.reply(&msg).await?;
            return Err(msg.into());
        }
        if longest.0 < pn.len() {
            longest = (pn.len(), pn.clone());
        }
    }
    ctx.data()
        .read()
        .await
        .get(&ctx.guild_id().unwrap())
        .unwrap()
        .read()
        .await
        .users
        .get(&ctx.author().id)
        .unwrap()
        .write()
        .await
        .pronouns = pronouns;
    write_cfg_file(ctx).await?;
    let change_nickname = ctx
        .data()
        .read()
        .await
        .get(&ctx.guild_id().unwrap())
        .unwrap()
        .read()
        .await
        .users
        .get(&ctx.author().id)
        .unwrap()
        .read()
        .await
        .change_nickname;
    let cur_name = ctx
        .partial_guild()
        .await
        .unwrap()
        .member(ctx, ctx.author().id)
        .await
        .unwrap()
        .display_name()
        .to_string();
    if change_nickname && cur_name.len() + 3 + longest.0 > 32 {
        ctx.reply(format!(
            concat!(
                "Pronouns successfully changed! Warning: you asked me to change your nickname, but ",
                "the longest pronouns in your list (`{}`) combined with your nickname (`{}`) is {} ",
                "characters over the limit. I will not attempt to modify your nickname until this ",
                "is resolved.",
            ),
            longest.1,
            cur_name,
            cur_name.len() + 3 + longest.0 - 32,
        ))
        .await?;
    } else {
        ctx.reply("Registration successful!").await?;
    }
    Ok(())
}

async fn write_cfg_file(ctx: Context<'_>) -> Result<(), Error> {
    let new_cfg = {
        let read = ctx.data().read().await;
        let mut new_cfg = Config {
            servers: Servers(HashMap::with_capacity(read.len())),
        };
        for (gid, scfg) in read.iter() {
            let read_lock = scfg.read().await;
            let mut un_rwlocked = HashMap::with_capacity(read_lock.users.len());
            for (uid, rwlock) in read_lock.users.iter() {
                let _ = un_rwlocked.insert(*uid, rwlock.read().await.clone());
            }
            let cfg = ServerConfig {
                update_channel: read_lock.update_channel,
                users: un_rwlocked,
            };
            let _ = new_cfg.servers.0.insert(*gid, cfg);
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
            let read_lock = scfg.read().await;
            let mut un_rwlocked = HashMap::with_capacity(read_lock.users.len());
            for (uid, rwlock) in read_lock.users.iter() {
                let _ = un_rwlocked.insert(*uid, rwlock.read().await.clone());
            }
            let cfg = ServerConfig {
                update_channel: read_lock.update_channel,
                users: un_rwlocked,
            };
            let _ = new_cfg.servers.0.insert(*gid, cfg);
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
