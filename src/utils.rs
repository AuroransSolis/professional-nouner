use crate::{
    server_data::{ServerConfig, ServerConfigCtx},
    user_data::UserData,
};
use poise::CreateReply;
use serde::{Deserialize, Serialize};
use serenity::{
    all::prelude::CacheHttp,
    builder::EditMember,
    model::{guild::Member, id::GuildId},
};
use std::{collections::HashMap, fmt::Write, io::Error as IoError, sync::Arc};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::RwLock};

pub type Data = Arc<RwLock<HashMap<GuildId, RwLock<ServerConfigCtx>>>>;
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Data, Error>;

pub trait GuildIdExt {
    fn embed_guild_name(&self, ctx: Context<'_>) -> String;
}

impl GuildIdExt for GuildId {
    fn embed_guild_name(&self, ctx: Context<'_>) -> String {
        self.name(ctx).unwrap_or_else(|| format!("GID: {self}"))
    }
}

pub trait RwLockUserDataExt {
    async fn user_pronouns_string(&self) -> String;
}

impl RwLockUserDataExt for RwLock<UserData> {
    async fn user_pronouns_string(&self) -> String {
        self.read()
            .await
            .pronouns
            .iter()
            .map(String::as_str)
            .intersperse(", ")
            .collect::<String>()
    }
}

pub async fn parse_pronouns(ctx: Context<'_>, pronouns: String) -> Result<Vec<String>, Error> {
    let pronouns = pronouns.split(',').map(str::to_string).collect::<Vec<_>>();
    let mut longest = (0, String::new());
    for pn in &pronouns {
        if !pn.chars().all(|c| c.is_alphabetic() || c == '/') {
            let msg = format!("Pronoun `{pn}` contains non-alphabetic character that is not `/`!");
            ctx.reply(&msg).await?;
            return Err(msg.into());
        }
        if pn.len() > 10 {
            let msg = format!("Pronoun `{pn}` has length exceeding maximum (10)!");
            ctx.reply(&msg).await?;
            return Err(msg.into());
        }
        if longest.0 < pn.len() {
            longest = (pn.len(), pn.clone());
        }
    }
    Ok(pronouns)
}

pub async fn clean_member_nickname<T: CacheHttp>(
    cache_http: T,
    member: &mut Member,
    current: &str,
) -> Result<&'static str, Error> {
    if let Some(cleaned) = member
        .display_name()
        .strip_suffix(current)
        .and_then(|cleaned| cleaned.strip_suffix(" | "))
    {
        member
            .edit(cache_http, EditMember::new().nickname(cleaned))
            .await?;
    }
    Ok("Successfully cleaned your nickname.")
}

pub async fn update_member_nickname<T: CacheHttp>(
    cache_http: T,
    member: &mut Member,
    data: &UserData,
    old: &str,
) -> Result<String, Error> {
    let mut msg = String::new();
    let cur_dn = member.display_name();
    let cleaned = member
        .display_name()
        .strip_suffix(old)
        .and_then(|cleaned| cleaned.strip_suffix(" | "))
        .unwrap_or(cur_dn);
    let mut longest = (0, String::new());
    for pn in &data.pronouns {
        if pn.len() > longest.0 {
            longest = (pn.len(), pn.clone());
        }
    }
    if cleaned.len() + longest.0 + 3 > 32 {
        writeln!(
            msg,
            concat!(
                "Warning: your current name plus your longest pronouns (`{}`) are {} ",
                "characters too long. I will only attempt to modify your nickname if it ",
                "doesn't require truncation.",
            ),
            longest.1,
            cleaned.len() + longest.0 + 3 - 32,
        )
        .unwrap();
    }
    let new = format!("{cleaned} | {}", data.current_pronoun());
    if new.len() <= 32 {
        member
            .edit(cache_http, EditMember::new().nickname(new))
            .await?;
        writeln!(msg, "Successfully changed your current nickname.").unwrap();
    } else {
        writeln!(
            msg,
            concat!(
                "Warning: your current name plus your current pronouns (`{}`) are {} ",
                "characters too long. I will only attempt to modify your nickname if it ",
                "doesn't require truncation.",
            ),
            data.current_pronoun(),
            cleaned.len() + data.current_pronoun().len() + 3 - 32,
        )
        .unwrap();
    }
    msg = msg.trim().to_string();
    Ok(msg)
}

pub async fn update_member<T: CacheHttp>(
    cache_http: T,
    member: &mut Member,
    data: &UserData,
    old: &str,
) -> Result<String, Error> {
    if data.change_nickname {
        update_member_nickname(cache_http, member, data, old).await
    } else {
        clean_member_nickname(cache_http, member, data.current_pronoun())
            .await
            .map(ToString::to_string)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub servers: Servers,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Servers(pub HashMap<GuildId, ServerConfig>);

pub async fn ctx_data_to_config(data: &Data) -> Config {
    let read = data.read().await;
    let mut cfg = Config {
        servers: Servers(HashMap::with_capacity(read.len())),
    };
    for (gid, scfgctx) in read.iter() {
        let read_lock = scfgctx.read().await;
        let mut un_rwlocked = HashMap::with_capacity(read_lock.users.len());
        for (uid, rwlock) in &read_lock.users {
            let _ = un_rwlocked.insert(*uid, rwlock.read().await.clone());
        }
        let scfg = ServerConfig {
            update_channel: read_lock.update_channel,
            users: un_rwlocked,
        };
        let _ = cfg.servers.0.insert(*gid, scfg);
    }
    cfg
}

#[derive(Debug)]
pub enum CfgError {
    OnOpen(IoError),
    OnWrite(IoError),
}

pub async fn write_cfg_file_noreply(data: &Data) -> Result<(), CfgError> {
    let data = ctx_data_to_config(data).await;
    let formatted = toml::to_string_pretty(&data).unwrap();
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(concat!(env!("CARGO_MANIFEST_DIR"), "/data.toml"))
        .await
        .map_err(|err| {
            println!("Error opening data file for writing: {err}");
            CfgError::OnOpen(err)
        })?;
    file.write_all(formatted.as_bytes()).await.map_err(|err| {
        println!("Error writing updated data to file: {err}");
        CfgError::OnWrite(err)
    })?;
    println!("Wrote new config to file.");
    Ok(())
}

pub async fn write_cfg_file(ctx: Context<'_>) -> Result<(), Error> {
    match write_cfg_file_noreply(ctx.data()).await {
        Ok(()) => Ok(()),
        Err(CfgError::OnOpen(err) | CfgError::OnWrite(err)) => {
            ctx.send(
                CreateReply::default()
                    .content("Bot host failed to save changes! Please retry.")
                    .ephemeral(true),
            )
            .await?;
            Err(err.into())
        }
    }
}
