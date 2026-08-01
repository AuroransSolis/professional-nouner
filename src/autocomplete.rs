use crate::Context;
use fuzzy_matcher::FuzzyMatcher;

pub async fn channels(ctx: Context<'_>, partial: &str) -> impl Iterator<Item = String> {
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

pub async fn registered_guilds(ctx: Context<'_>, partial: &str) -> impl Iterator<Item = String> {
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

pub async fn registered_users(ctx: Context<'_>, partial: &str) -> impl Iterator<Item = String> {
    let Some(gid) = ctx.guild_id() else {
        return Vec::new().into_iter();
    };
    let mut users = Vec::new();
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
    for user_id in ctx
        .data()
        .read()
        .await
        .get(&gid)
        .unwrap()
        .read()
        .await
        .users
        .keys()
    {
        let Ok(member) = gid.member(ctx, *user_id).await else {
            continue;
        };
        let display_name = member.display_name();
        if matcher.fuzzy_match(display_name, partial).is_some() {
            users.push(display_name.to_string());
        }
    }
    users.into_iter()
}
