use crate::utils::{Data, Error, update_member, write_cfg_file_noreply};
use chrono::{Timelike, Utc};
use serenity::{
    builder::{CreateEmbed, CreateMessage, EditMember},
    client::Cache,
    http::Http,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub async fn updater_loop(
    die: Arc<AtomicBool>,
    bot_data: Data,
    http: Arc<Http>,
    cache: Arc<Cache>,
) -> Result<(), Error> {
    while !die.load(Ordering::Relaxed) {
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
        println!("Sleeping for: {until_midnight:?}");
        tokio::time::sleep(until_midnight).await;
        println!("Running daily update");
        for (guild_id, guild_cfg) in bot_data.read().await.iter() {
            let Some(channel_id) = guild_cfg.read().await.update_channel else {
                for (uid, user_data) in guild_cfg.read().await.users.iter() {
                    let mut lock = user_data.write().await;
                    if lock.pronouns.len() == 1 {
                        continue;
                    }
                    let old = lock.current_and_reroll();
                    let data = lock.clone();
                    let Ok(mut member) = guild_id.member((&cache, http.as_ref()), uid).await else {
                        continue;
                    };
                    let _ = update_member((&cache, http.as_ref()), &mut member, &data, &old).await;
                }
                continue;
            };
            if guild_cfg.read().await.users.is_empty() {
                continue;
            }
            let mut user_data = Vec::new();
            for (user_id, ud) in &guild_cfg.read().await.users {
                let mut ud_lock = ud.write().await;
                let prev = ud_lock.current_and_reroll();
                user_data.push((
                    *user_id,
                    prev,
                    ud_lock.current_pronoun().to_string(),
                    ud_lock.change_nickname,
                ));
            }
            let mut fields = Vec::with_capacity(user_data.len());
            let mut too_long = Vec::new();
            let mut permissions = Vec::new();
            for (id, prev_pronoun, current_pronoun, change_nickname) in user_data {
                let name = match http.get_member(*guild_id, id).await {
                    Ok(mut member) => {
                        let name = member.display_name().to_string();
                        if change_nickname {
                            let cleaned = name
                                .strip_suffix(&prev_pronoun)
                                .and_then(|stripped| stripped.strip_suffix(" | "))
                                .unwrap_or(name.as_str());
                            if cleaned.len() + current_pronoun.len() + 3 <= 32 {
                                let name = format!("{cleaned} | {current_pronoun}");
                                if member
                                    .edit((&cache, http.as_ref()), EditMember::new().nickname(name))
                                    .await
                                    .is_err()
                                {
                                    permissions.push(cleaned.to_string());
                                }
                            } else {
                                too_long.push(name.clone());
                            }
                            cleaned.to_string()
                        } else {
                            name
                        }
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
            let msg = CreateMessage::new().embed(
                CreateEmbed::new()
                    .title("The Pronoun Update")
                    .description("Here's the new pronouns for the registered users in this server!")
                    .fields(fields),
            );
            if let Err(err) = channel_id.send_message((&cache, http.as_ref()), msg).await {
                println!("Failed to send registrar update to `{guild_id}/{channel_id}`: {err:?}");
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
    Ok(())
}
