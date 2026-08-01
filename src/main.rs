#![feature(iter_intersperse)]
#![allow(clippy::too_many_lines, clippy::unused_async)]

mod autocomplete;
mod check;
mod deregister;
mod register;
mod reroll;
mod server_data;
mod settings;
mod updater_loop;
mod user_data;
mod utils;

use poise::CreateReply;
use serenity::{
    Client,
    all::{
        GatewayIntents, GuildId,
        prelude::{EventHandler, TypeMapKey},
    },
    async_trait,
    builder::{CreateEmbed, EditMember},
    model::{
        guild::{Guild, Member, UnavailableGuild},
        user::{OnlineStatus, User},
    },
};
use server_data::ServerConfigCtx;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::RwLock;
use updater_loop::updater_loop;
use utils::{Config, Context, Data, Error, write_cfg_file, write_cfg_file_noreply};

use crate::utils::update_member_nickname;

struct Helper;

impl TypeMapKey for Helper {
    type Value = Data;
}

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
                deregister::deregister(),
                register::register(),
                registrar(),
                reroll::reroll(),
                set_announce_channel(),
                settings::settings(),
            ],
            on_error: |err| {
                Box::pin(async move {
                    let Some(ctx) = err.ctx() else {
                        println!("Error with no context:\n{err:?}");
                        return;
                    };
                    println!(
                        "Command {:?} triggered by {} produced error: {err:?}",
                        ctx.command().qualified_name,
                        ctx.author().name
                    );
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
                        poise::FrameworkError::CommandCheckFailed {
                            error: Some(error),
                            ctx,
                            ..
                        } => {
                            let _ = ctx
                                .send(CreateReply::default().content(format!(
                                    "Oops! Looks like you failed a command check. Error: {error}"
                                )).ephemeral(false))
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
                })
            },
            ..Default::default()
        })
        .setup(|ctx, _, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                let unavailable = ctx.cache.unavailable_guilds();
                let mut modified = false;
                for (gid, server_cfg) in init_data
                    .read()
                    .await
                    .iter()
                    .filter(|(gid, _)| unavailable.get(gid).is_none())
                {
                    let mut remove_userids = Vec::new();
                    for (uid, user_data) in &server_cfg.read().await.users {
                        match gid.member(ctx, uid).await {
                            Ok(mut member) => {
                                let user_data = user_data.read().await;
                                if !user_data.change_nickname {
                                    continue;
                                }
                                let display_name = member.display_name();
                                if let Some((idx, cleaned)) =
                                    user_data.pronouns.iter().enumerate().find_map(|(i, pn)| {
                                        display_name
                                            .strip_suffix(pn)
                                            .and_then(|s| s.strip_suffix(" | "))
                                            .map(|s| (i, s))
                                    })
                                {
                                    // Only attempt to fix the suffix if the index is incorrect
                                    if idx != user_data.current {
                                        let name =
                                            format!("{cleaned} | {}", user_data.current_pronoun());
                                        member.edit(ctx, EditMember::new().nickname(name)).await?;
                                    }
                                } else {
                                    // Somehow lacking a suffix, add it
                                    update_member_nickname(ctx, &mut member, &user_data, "").await?;
                                }
                            }
                            Err(serenity::Error::Http(_)) => {
                                // Member is not in guild, should be removed.
                                println!(
                                    "    User `{uid}` is not in guild `{gid}` anymore, removing"
                                );
                                remove_userids.push(*uid);
                            }
                            Err(_) => (), // Other error, ignore.
                        }
                    }
                    if !remove_userids.is_empty() {
                        let mut lock = server_cfg.write().await;
                        for user_id in remove_userids {
                            let _ = lock.users.remove(&user_id);
                        }
                        modified = true;
                    }
                }
                if modified {
                    write_cfg_file_noreply(&init_data).await.unwrap();
                }
                Ok(init_data)
            })
        })
        .build();

    let mut client = Client::builder(token, intents)
        .type_map_insert::<Helper>(bot_data.clone())
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
                .for_each(|(_, v)| v.runner_tx.set_status(OnlineStatus::Offline));
        });
        shard_manager.shutdown_all().await;
    });

    let http = client.http.clone();
    let cache = client.cache.clone();
    tokio::spawn(updater_loop(die1, bot_data, http, cache));

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

    async fn guild_create(
        &self,
        ctx: poise::serenity_prelude::Context,
        guild: Guild,
        _: Option<bool>,
    ) {
        let lock = ctx.data.read().await;
        let poise_data = lock.get::<Helper>().unwrap();
        if !poise_data.read().await.contains_key(&guild.id) {
            poise_data.write().await.insert(
                guild.id,
                RwLock::new(ServerConfigCtx {
                    update_channel: None,
                    users: HashMap::new(),
                }),
            );
            let _ = write_cfg_file_noreply(poise_data).await;
        }
    }

    async fn guild_delete(
        &self,
        ctx: poise::serenity_prelude::Context,
        incomplete: UnavailableGuild,
        _: Option<Guild>,
    ) {
        if !incomplete.unavailable {
            // Bot was removed from guild, remove that guild's data
            let lock = ctx.data.read().await;
            let poise_data = lock.get::<Helper>().unwrap();
            let _ = poise_data.write().await.remove(&incomplete.id);
            let _ = write_cfg_file_noreply(poise_data).await;
        }
    }

    async fn guild_member_removal(
        &self,
        ctx: poise::serenity_prelude::Context,
        guild_id: GuildId,
        user: User,
        _: Option<Member>,
    ) {
        let lock = ctx.data.read().await;
        let poise_data = lock.get::<Helper>().unwrap();
        let lock = poise_data.read().await;
        let Some(server_cfg) = lock.get(&guild_id) else {
            return;
        };
        let mut lock = server_cfg.write().await;
        let _ = lock.users.remove(&user.id);
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
    interaction_context = "Guild"
)]
/// Shows the current listing of pronouns for registered users in this server
async fn registrar(
    ctx: Context<'_>,
    #[description = "Fetch current pronouns of a specific user"]
    #[autocomplete = "autocomplete::registered_users"]
    user: Option<User>,
    #[description = "Make this reply ephemeral"] ephemeral: Option<bool>,
) -> Result<(), Error> {
    println!(
        "Accessing the registrar for server `{}`",
        ctx.guild_id().unwrap()
    );
    let ephemeral = ephemeral.unwrap_or(true);
    let guild = ctx.guild().unwrap().clone();
    if let Some(user) = user {
        let current = ctx
            .data()
            .read()
            .await
            .get(&guild.id)
            .unwrap()
            .read()
            .await
            .users
            .get(&user.id)
            .unwrap()
            .read()
            .await
            .current_pronoun()
            .to_string();
        let display_name = guild
            .member(ctx, user.id)
            .await
            .unwrap()
            .display_name()
            .to_string();
        let display_name = display_name
            .strip_suffix(&current)
            .and_then(|stripped| stripped.strip_suffix(" | "))
            .map(ToString::to_string)
            .unwrap_or(display_name);
        ctx.send(
            CreateReply::default()
                .content(format!(
                    "**{display_name}**'s pronouns are currently **{current}**."
                ))
                .ephemeral(ephemeral),
        )
        .await?;
    } else {
        let mut user_data = Vec::new();
        for (user_id, ud) in &ctx
            .data()
            .read()
            .await
            .get(&ctx.guild_id().unwrap())
            .unwrap()
            .read()
            .await
            .users
        {
            let ud_lock = ud.read().await;
            user_data.push((*user_id, ud_lock.current_pronoun().to_string()));
        }
        let mut fields = Vec::with_capacity(user_data.len());
        for (id, current_pronoun) in user_data {
            let member = guild.member(ctx, id).await?;
            let name = member
                .display_name()
                .strip_suffix(&current_pronoun)
                .and_then(|stripped| stripped.strip_suffix(" | "))
                .unwrap_or(member.display_name());
            fields.push((name.to_string(), current_pronoun, false));
        }
        fields.sort_by(|a, b| {
            let mut ac = a.0.chars();
            let mut bc = b.0.chars();
            loop {
                match (ac.next(), bc.next()) {
                    (Some(_), None) => return std::cmp::Ordering::Greater,
                    (None, Some(_)) => return std::cmp::Ordering::Less,
                    (None, None) => return std::cmp::Ordering::Equal,
                    (Some(a), Some(b)) => match a.to_lowercase().cmp(b.to_lowercase()) {
                        std::cmp::Ordering::Equal => (),
                        other => return other,
                    },
                }
            }
        });
        ctx.send(
            CreateReply::default()
                .embed(CreateEmbed::new().title("Registrar").fields(fields))
                .ephemeral(ephemeral),
        )
        .await?;
    }
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "announcement",
    install_context = "Guild",
    interaction_context = "Guild"
)]
/// Set the channel to send the pronouns update in for this server
async fn set_announce_channel(
    ctx: Context<'_>,
    #[description = "Set the channel to send the daily update in. None for no announcements."]
    #[autocomplete = "autocomplete::channels"]
    channel: Option<poise::serenity_prelude::GuildChannel>,
) -> Result<(), Error> {
    println!(
        "Setting announcement channel for server `{}` to `{:?}`",
        ctx.guild_id().unwrap(),
        channel.as_ref().map(|gc| gc.id),
    );
    if let Some(guild_channel) = channel {
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
                .update_channel = Some(channel_id);
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
    } else {
        ctx.data()
            .read()
            .await
            .get(&ctx.guild_id().unwrap())
            .unwrap()
            .write()
            .await
            .update_channel = None;
        write_cfg_file(ctx).await?;
        ctx.reply("Cleared announcement channel! I won't make announcements without one set.")
            .await?;
        Ok(())
    }
}
