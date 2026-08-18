use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::resources::{ResourceCondition, ResourceRef};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthObservation {
    pub resource: ResourceRef,
    pub observed_at: DateTime<Utc>,
    pub state: HealthState,
    pub started_at: Option<DateTime<Utc>>,
    pub ready_since: Option<DateTime<Utc>>,
    pub restart_count: Option<u32>,
    pub conditions: Vec<ResourceCondition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
    Pending,
    Terminating,
    Unknown,
}
