use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::resources::ResourceRef;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub resource: ResourceRef,
    pub timestamp: Option<DateTime<Utc>>,
    pub container: Option<String>,
    pub stream: LogStream,
    pub message: String,
    pub previous_container: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
    Unknown,
}
