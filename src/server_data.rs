use crate::user_data::UserData;
use serde::{Deserialize, Serialize};
use serenity::model::id::{ChannelId, UserId};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    pub update_channel: Option<ChannelId>,
    #[serde(flatten)]
    pub users: HashMap<UserId, UserData>,
}

#[derive(Debug)]
pub struct ServerConfigCtx {
    pub update_channel: Option<ChannelId>,
    pub users: HashMap<UserId, RwLock<UserData>>,
}
