mod nickname;
mod pronouns;

use crate::{Context, Error};

#[poise::command(
    slash_command,
    subcommands(
        "nickname::settings_cn_get_server",
        "nickname::settings_cn_get_global",
        "nickname::settings_cn_set_server",
        "nickname::settings_cn_set_global",
        "pronouns::settings_pn_get_server",
        "pronouns::settings_pn_get_global",
        "pronouns::settings_pn_set_server",
        "pronouns::settings_pn_set_global",
    ),
    subcommand_required,
    // install_context = "Guild",
    // interaction_context = "Guild"
)]
pub async fn settings(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

// #[poise::command(
//     slash_command,
//     rename = "nickname",
//     subcommands(
//         "nickname::settings_cn_get_server",
//         "nickname::settings_cn_get_global",
//         "nickname::settings_cn_set_server",
//         "nickname::settings_cn_set_global",
//     ),
//     subcommand_required = true,
//     // install_context = "Guild",
//     // interaction_context = "Guild",
//     // check = "crate::check::user_is_registered",
//     // ephemeral = true
// )]
// /// Get/set whether the bot should attempt to change your nickname on registrar updates
// pub async fn settings_cn(
//     _: Context<'_>,
//     // #[description = "Allow this bot to attempt to change your nickname on registrar updates."]
//     // change_nickname: Option<bool>,
// ) -> Command<> {
//     // match change_nickname {
//     // Some(set) => nickname::settings_cn_set(ctx, set).await,
//     // None => settings_cn_get(ctx).await,
//     // }
//     Ok(())
// }

// #[poise::command(
//     slash_command,
//     rename = "pronouns",
//     subcommands(
//         "pronouns::settings_pn_get_server",
//         "pronouns::settings_pn_get_global",
//         "pronouns::settings_pn_set_server",
//         "pronouns::settings_pn_set_global",
//     ),
//     subcommand_required = true,
//     // install_context = "Guild",
//     // interaction_context = "Guild",
//     // check = "crate::check::user_is_registered",
//     // ephemeral = true
// )]
// // /// Get/set your pronouns
// pub async fn settings_pn(
//     _: Context<'_>,
//     // #[description = "Separate groups with commas, each must be alphabetic and under 10 chars."]
//     // #[min_length = 1]
//     // pronouns: Option<String>,
// ) -> Result<(), Error> {
//     // match pronouns {
//     // Some(set) => pronouns::settings_pn_set(ctx, set).await,
//     // None => pronouns::settings_pn_get(ctx).await,
//     // }
//     Ok(())
// }
