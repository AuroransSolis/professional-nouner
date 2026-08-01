use poise::CreateReply;
use serenity::builder::CreateEmbed;

use crate::{
    Context, Error, utils::{GuildIdExt, update_member, write_cfg_file},
};

#[poise::command(
    slash_command,
    subcommands("reroll_local", "reroll_global"),
    subcommand_required
)]
pub async fn reroll(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(
    slash_command,
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::user_is_registered",
    ephemeral = true,
    member_cooldown = 10
)]
/// Reroll your pronouns in this server
async fn reroll_local(ctx: Context<'_>) -> Result<(), Error> {
    println!(
        "Rerolling pronouns for user `{}` in server `{}`",
        ctx.author().id,
        ctx.guild_id().unwrap()
    );
    let (old, user_data) = {
        let lock = ctx.data().read().await;
        let lock = lock.get(&ctx.guild_id().unwrap()).unwrap().read().await;
        let mut lock = lock.users.get(&ctx.author().id).unwrap().write().await;
        let old = lock.current_and_reroll();
        (old, lock.clone())
    };
    write_cfg_file(ctx).await?;
    if user_data.change_nickname {
        let mut member = ctx.guild_id().unwrap().member(ctx, ctx.author().id).await?;
        let mut msg = update_member(ctx, &mut member, &user_data, &old).await?;
        msg.push_str("\nSuccessfully rerolled your pronouns and applied name change.");
        ctx.reply(msg).await?;
    } else {
        ctx.reply("Successfully rerolled your pronouns.").await?;
    }
    Ok(())
}

#[poise::command(
    slash_command,
    install_context = "User",
    interaction_context = "BotDm",
    check = "crate::check::user_is_registered_anywhere",
    ephemeral = true,
    user_cooldown = 10
)]
async fn reroll_global(ctx: Context<'_>) -> Result<(), Error> {
    println!("Rerolling pronouns for user `{}` globally", ctx.author().id);
    let mut fields = Vec::new();
    for (gid, server_cfg) in ctx.data().read().await.iter() {
        let lock = server_cfg.read().await;
        let Some(user_cfg) = lock.users.get(&ctx.author().id) else {
            continue;
        };
        let (old, user_data) = {
            let mut lock = user_cfg.write().await;
            let old = lock.current_and_reroll();
            (old, lock.clone())
        };
        if user_data.change_nickname {
            let mut member = match gid.member(ctx, ctx.author().id).await {
                Ok(member) => member,
                Err(err) => {
                    let gn = gid.embed_guild_name(ctx);
                    println!("    Failed to get member data in guild `{gn}`");
                    fields.push((gn, format!("Failed to get member data: {err}"), false));
                    continue;
                }
            };
            match update_member(ctx, &mut member, &user_data, &old).await {
                Ok(ref msg)
                    if let Some(stripped) =
                        msg.strip_suffix("Successfully changed your current nickname.")
                        && !stripped.is_empty() =>
                {
                    fields.push((gid.embed_guild_name(ctx), stripped.to_string(), false));
                }
                Ok(_) => (),
                Err(err) => {
                    let gn = gid.embed_guild_name(ctx);
                    println!("    Failed to update member nickname in guild `{gn}`");
                    fields.push((gn, format!("Failed to update display name: {err}"), false));
                }
            }
        }
    }
    write_cfg_file(ctx).await?;
    if fields.is_empty() {
        ctx.reply("Successfully rerolled your pronouns in all registered servers.")
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
