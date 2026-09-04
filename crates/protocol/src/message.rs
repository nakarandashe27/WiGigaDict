use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: &str = "0.2.0";
pub const MAX_NDJSON_LINE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Shell,
    Sidecar,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    #[serde(rename = "type")]
    pub message_type: String,
    pub protocol: String,
    pub app: String,
    pub commit: String,
    pub role: Role,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HelloAck {
    #[serde(rename = "type")]
    pub message_type: String,
    pub protocol: String,
    pub app: String,
    pub commit: String,
    pub role: Role,
}
