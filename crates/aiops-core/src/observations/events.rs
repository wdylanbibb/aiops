use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::resources::ResourceRef;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEvent {
    pub regarding: ResourceRef,
    pub reporting_controller: Option<String>,
    pub event_type: EventType,
    pub reason: Option<String>,
    pub message: String,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
    pub count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Normal,
    Warning,
    Unknown,
}
