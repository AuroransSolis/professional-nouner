use crate::{
    Context, Error,
    utils::{GuildIdExt, update_member, write_cfg_file},
};
use poise::CreateReply;
use serenity::builder::CreateEmbed;

#[poise::command(
    slash_command,
    rename = "change_nickname_get_local",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::user_is_registered",
    ephemeral = true
)]
pub async fn settings_cn_get_server(ctx: Context<'_>) -> Result<(), Error> {
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

#[poise::command(
    slash_command,
    rename = "change_nickname_get_global",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::user_is_registered_anywhere",
    ephemeral = true
)]
pub async fn settings_cn_get_global(ctx: Context<'_>) -> Result<(), Error> {
    println!(
        "Fetching `change_nickname` for user `{}` in globally",
        ctx.author().id,
    );
    let mut fields = Vec::new();
    for (gid, cfg) in ctx.data().read().await.iter() {
        let guild_name = gid.embed_guild_name(ctx);
        let lock = cfg.read().await;
        let Some(cfg) = lock.users.get(&ctx.author().id) else {
            continue;
        };
        let enabled = if cfg.read().await.change_nickname {
            "Nickname changing enabled"
        } else {
            "Nickname changing disabled"
        };
        fields.push((guild_name, enabled, false));
    }
    ctx.send(CreateReply::default().embed(CreateEmbed::new().fields(fields)))
        .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "change_nickname_set_local",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::user_is_registered",
    ephemeral = true
)]
pub async fn settings_cn_set_server(
    ctx: Context<'_>,
    #[description = "Allow this bot to attempt to change your nickname on registrar updates."]
    change_nickname: bool,
) -> Result<(), Error> {
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
    let (changed, cur_data) = {
        let lock = ctx.data().read().await;
        let lock = lock.get(&ctx.guild_id().unwrap()).unwrap().read().await;
        let mut lock = lock.users.get(&ctx.author().id).unwrap().write().await;
        let changed = change_nickname ^ lock.change_nickname;
        lock.change_nickname = change_nickname;
        (changed, lock.clone())
    };
    if !changed {
        return Ok(());
    }
    write_cfg_file(ctx).await?;
    let mut member = ctx
        .guild_id()
        .unwrap()
        .member(ctx, ctx.author().id)
        .await
        .unwrap();
    let msg = update_member(ctx, &mut member, &cur_data, "").await?;
    ctx.reply(msg).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "change_nickname_set_global",
    install_context = "User",
    interaction_context = "BotDm",
    check = "crate::check::user_is_registered_anywhere",
    ephemeral = true
)]
pub async fn settings_cn_set_global(
    ctx: Context<'_>,
    #[description = "Allow this bot to attempt to change your nickname on registrar updates."]
    change_nickname: bool,
) -> Result<(), Error> {
    println!(
        "Changing `change_nickname` for user `{}` globally",
        ctx.author().id,
    );
    let mut any_changed = false;
    let mut fields = Vec::new();
    for (gid, server_cfg) in ctx.data().read().await.iter() {
        if let Some(guild) = gid.to_guild_cached(&ctx)
            && guild.owner_id == ctx.author().id
        {
            continue;
        }
        let (changed, cur_data) = {
            let lock = server_cfg.read().await;
            let Some(lock) = lock.users.get(&ctx.author().id) else {
                continue;
            };
            let mut lock = lock.write().await;
            let changed = lock.change_nickname ^ change_nickname;
            lock.change_nickname = change_nickname;
            (changed, lock.clone())
        };
        if !changed {
            continue;
        }
        any_changed = true;
        let Ok(mut member) = gid.member(ctx, ctx.author().id).await else {
            continue;
        };
        let guild_name = gid.embed_guild_name(ctx);
        let msg = match update_member(ctx, &mut member, &cur_data, "").await {
            Ok(msg) => msg,
            Err(err) => {
                format!("Failed to update member data in guild `{guild_name}`. Error: {err}")
            }
        };
        fields.push((guild_name, msg, false));
    }
    if any_changed {
        write_cfg_file(ctx).await?;
    }
    if fields.is_empty() {
        ctx.reply("Successfully set `change_nickname` to `{change_nickname}` in all servers")
            .await?;
    } else {
        ctx.send(
            CreateReply::default().embed(
                CreateEmbed::new()
                    .title("Warnings and errors")
                    .fields(fields),
            ),
        )
        .await?;
    }
    Ok(())
}
