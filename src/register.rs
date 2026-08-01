use crate::{
    Context, Error,
    user_data::UserData,
    utils::{parse_pronouns, update_member, write_cfg_file},
};
use getrandom::SysRng;
use rand::{RngExt, rand_core::UnwrapErr};
use serenity::model::guild::Guild;
use tokio::sync::RwLock;

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
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::user_not_registered",
    ephemeral = true
)]
/// Copy your pronouns from another server to the one you send this command in
async fn register_copy(
    ctx: Context<'_>,
    #[description = "Server to copy from"]
    #[autocomplete = "crate::autocomplete::registered_guilds"]
    guild: Guild,
) -> Result<(), Error> {
    println!(
        "Registering user `{}` by copying from guild `{}`",
        ctx.author().id,
        guild.id
    );
    let lock = ctx.data().read().await;
    lock.get(&ctx.guild_id().unwrap())
        .unwrap()
        .write()
        .await
        .users
        .insert(
            ctx.author().id,
            RwLock::new({
                let mut data = lock
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
                data.change_nickname &= ctx.guild().unwrap().owner_id != ctx.author().id;
                data
            }),
        );
    write_cfg_file(ctx).await?;
    ctx.reply("Registration successful!").await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "new",
    install_context = "Guild",
    interaction_context = "Guild",
    check = "crate::check::user_not_registered",
    ephemeral = true
)]
/// Register with this server, and provide a list of pronouns to use
async fn register_new(
    ctx: Context<'_>,
    #[description = "Should I try and append your pronouns to your nickname when they're updated?"]
    change_nickname: Option<bool>,
    #[description = "Pronouns to register with. Separate with commas, must be alphabetic and under 10 chars."]
    #[min_length = 1]
    pronouns: String,
) -> Result<(), Error> {
    println!("Registering user `{}` with new data", ctx.author().id);
    let change_nickname =
        change_nickname.unwrap_or(false) && ctx.guild().unwrap().owner_id != ctx.author().id;
    let pronouns = parse_pronouns(ctx, pronouns).await?;
    let user_data = UserData {
        current: if pronouns.len() == 1 {
            0
        } else {
            UnwrapErr(SysRng).random_range(0..pronouns.len())
        },
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
        .insert(ctx.author().id, RwLock::new(user_data.clone()));
    write_cfg_file(ctx).await?;
    if user_data.change_nickname {
        let mut member = match ctx.guild_id().unwrap().member(ctx, ctx.author().id).await {
            Ok(member) => member,
            Err(err) => {
                println!("    Failed to retrieve member data to update nickname!");
                ctx.reply("Internal error while trying to update your nickname. Try `/reroll`ing.")
                    .await?;
                return Err(err.into());
            }
        };
        ctx.reply(update_member(ctx, &mut member, &user_data, "").await?)
            .await?;
    }
    Ok(())
}
