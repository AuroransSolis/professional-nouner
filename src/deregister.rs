use poise::CreateReply;
use serenity::builder::CreateEmbed;

use crate::{
    Context, Error,
    utils::{GuildIdExt, clean_member_nickname, write_cfg_file},
};

#[poise::command(
    slash_command,
    install_context = "Guild|User",
    subcommands("deregister_server", "deregister_global"),
    subcommand_required = true
)]
pub async fn deregister(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "server",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::guild_is_registered",
    check = "crate::check::user_is_registered",
    ephemeral = true
)]
/// Remove your user registration. In DMs removes all registrations, in server removes that server only.
async fn deregister_server(ctx: Context<'_>) -> Result<(), Error> {
    let user_id = ctx.author().id;
    let guild_id = ctx.guild_id().unwrap();
    println!("Deregistering user `{user_id}` in server `{guild_id}`");
    let user_data = ctx
        .data()
        .read()
        .await
        .get(&guild_id)
        .unwrap()
        .write()
        .await
        .users
        .remove(&user_id)
        .unwrap();
    write_cfg_file(ctx).await?;
    let data = user_data.into_inner();
    if data.change_nickname {
        let mut member = match guild_id.member(ctx, user_id).await {
            Ok(member) => member,
            Err(err) => {
                println!("  Failed to fetch member data: {err}");
                ctx.reply("Failed to fetch member data to clear nickname!")
                    .await?;
                return Err(
                    anyhow::anyhow!("Failed to fetch member data to clear nickname!").into(),
                );
            }
        };
        clean_member_nickname(ctx, &mut member, data.current_pronoun()).await?;
    }
    ctx.reply("Deregistered you in this guild!").await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "global",
    install_context = "User",
    interaction_context = "BotDm",
    check = "crate::check::user_is_registered_anywhere",
    ephemeral = true
)]
async fn deregister_global(ctx: Context<'_>) -> Result<(), Error> {
    let mut fields = Vec::new();
    let user_id = ctx.author().id;
    for (gid, server_cfg) in ctx.data().read().await.iter() {
        if let Some(user_data) = server_cfg.write().await.users.remove(&user_id) {
            write_cfg_file(ctx).await?;
            let data = user_data.into_inner();
            if data.change_nickname {
                let Ok(mut member) = gid.member(ctx, ctx.author().id).await else {
                    fields.push((
                        gid.embed_guild_name(ctx),
                        "Failed to fetch member data!".to_string(),
                        false,
                    ));
                    continue;
                };
                let Ok(_) = clean_member_nickname(ctx, &mut member, data.current_pronoun()).await
                else {
                    fields.push((
                        gid.embed_guild_name(ctx),
                        "Failed to clean display name!".to_string(),
                        false,
                    ));
                    continue;
                };
            }
        }
    }
    if fields.is_empty() {
        ctx.reply("Successfully deregistered you from all servers.")
            .await?;
    } else {
        ctx.send(CreateReply::default().embed(CreateEmbed::new().fields(fields)))
            .await?;
    }
    Ok(())
}
