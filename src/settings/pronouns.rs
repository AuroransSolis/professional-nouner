use crate::{
    Context, Error, utils::{
        GuildIdExt, RwLockUserDataExt, clean_member_nickname, parse_pronouns, update_member, write_cfg_file,
    },
};
use poise::CreateReply;
use serenity::builder::CreateEmbed;

#[poise::command(
    slash_command,
    rename = "pronouns_get_local",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::user_is_registered",
    ephemeral = true
)]
pub async fn settings_pn_get_server(ctx: Context<'_>) -> Result<(), Error> {
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
        .user_pronouns_string()
        .await;
    ctx.reply(format!("Your current pronouns are: `{current}`."))
        .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "pronouns_get_global",
    install_context = "User",
    interaction_context = "BotDm",
    check = "crate::check::user_is_registered_anywhere",
    ephemeral = true
)]
pub async fn settings_pn_get_global(ctx: Context<'_>) -> Result<(), Error> {
    println!(
        "Fetching pronouns for user `{}` in globally",
        ctx.author().id,
    );
    let mut fields = Vec::new();
    for (gid, server_cfg) in ctx.data().read().await.iter() {
        let lock = server_cfg.read().await;
        let Some(user_data) = lock.users.get(&ctx.author().id) else {
            continue;
        };
        let pronouns = user_data.user_pronouns_string().await;
        let guild_name = gid.embed_guild_name(ctx);
        fields.push((guild_name, pronouns, false));
    }
    ctx.send(CreateReply::default().embed(CreateEmbed::new().fields(fields)))
        .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "pronouns_set_local",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::user_is_registered",
    ephemeral = true
)]
pub async fn settings_pn_set_server(
    ctx: Context<'_>,
    #[description = "Separate groups with commas, each must be alphabetic and under 10 chars."]
    #[min_length = 1]
    pronouns: String,
) -> Result<(), Error> {
    println!(
        "Changing pronouns for user `{}` in server `{}`",
        ctx.author().id,
        ctx.guild_id().unwrap()
    );
    let pronouns = parse_pronouns(ctx, pronouns).await?;
    let (cur_data, new, new_data) = {
        let lock = ctx.data().read().await;
        let lock = lock.get(&ctx.guild_id().unwrap()).unwrap().read().await;
        let lock = lock.users.get(&ctx.author().id).unwrap();
        let mut lock = lock.write().await;
        let cur = lock.clone();
        lock.pronouns = pronouns;
        lock.current_sanity();
        (cur, lock.current_pronoun().to_string(), lock.clone())
    };
    write_cfg_file(ctx).await?;
    let mut member = ctx
        .guild_id()
        .unwrap()
        .member(ctx, ctx.author().id)
        .await
        .unwrap();
    let cur_pn = cur_data.current_pronoun();
    if cur_data.change_nickname && cur_pn != new && member.display_name().ends_with(cur_pn) {
        let _ = clean_member_nickname(ctx, &mut member, cur_pn).await;
        let mut msg = update_member(ctx, &mut member, &new_data, cur_pn).await?;
        msg.push_str("\nSuccessfully updated your pronouns.");
        ctx.reply(msg).await?;
    } else {
        ctx.reply("Successfully updated your pronouns!").await?;
    }
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "pronouns_set_global",
    install_context = "User",
    interaction_context = "BotDm",
    check = "crate::check::user_is_registered_anywhere",
    ephemeral = true
)]
pub async fn settings_pn_set_global(
    ctx: Context<'_>,
    #[description = "Separate groups with commas, each must be alphabetic and under 10 chars."]
    #[min_length = 1]
    pronouns: String,
) -> Result<(), Error> {
    println!(
        "Changing pronouns for user `{}` in globally",
        ctx.author().id,
    );
    let pronouns = parse_pronouns(ctx, pronouns).await?;
    let mut fields = Vec::new();
    for (gid, server_cfg) in ctx.data().read().await.iter() {
        let (cur_data, new, new_data) = {
            let lock = server_cfg.read().await;
            let Some(user_data) = lock.users.get(&ctx.author().id) else {
                continue;
            };
            let mut lock = user_data.write().await;
            let cur = lock.clone();
            lock.pronouns.clone_from(&pronouns);
            lock.current_sanity();
            (cur, lock.current_pronoun().to_string(), lock.clone())
        };
        let Ok(mut member) = gid.member(ctx, ctx.author().id).await else {
            continue;
        };
        let cur_pn = cur_data.current_pronoun();
        if cur_data.change_nickname && cur_pn != new && member.display_name().ends_with(cur_pn) {
            let msg = match update_member(ctx, &mut member, &new_data, cur_pn).await {
                Ok(msg) => msg,
                Err(err) => {
                    fields.push((gid.embed_guild_name(ctx), format!("{err}"), false));
                    continue;
                }
            };
            if let Some(rest) = msg.strip_suffix("Successfully changed your current nickname.")
                && !rest.is_empty()
            {
                fields.push((gid.embed_guild_name(ctx), rest.to_string(), false));
            }
        }
    }
    write_cfg_file(ctx).await?;
    if fields.is_empty() {
        ctx.reply("Successfully set pronouns in all servers")
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
