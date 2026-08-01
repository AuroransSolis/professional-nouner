use crate::{Context, Error};

pub async fn guild_is_registered(ctx: Context<'_>) -> Result<bool, Error> {
    if let Some(guild_id) = ctx.guild_id() {
        Ok(ctx.data().read().await.contains_key(&guild_id))
    } else {
        Ok(false)
    }
}

pub async fn user_is_registered(ctx: Context<'_>) -> Result<bool, Error> {
    if let Some(guild_id) = ctx.guild_id()
        && let Some(lock) = ctx.data().read().await.get(&guild_id)
    {
        Ok(lock.read().await.users.contains_key(&ctx.author().id))
    } else {
        Ok(false)
    }
}

pub async fn user_not_registered(ctx: Context<'_>) -> Result<bool, Error> {
    user_is_registered(ctx).await.map(std::ops::Not::not)
}

pub async fn user_is_registered_anywhere(ctx: Context<'_>) -> Result<bool, Error> {
    for cfg in ctx.data().read().await.values() {
        if cfg.read().await.users.contains_key(&ctx.author().id) {
            return Ok(true);
        }
    }
    Ok(false)
}
