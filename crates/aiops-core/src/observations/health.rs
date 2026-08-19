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
    pub containers: Vec<ContainerHealth>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerHealth {
    pub name: String,
    pub kind: ContainerKind,
    pub ready: bool,
    pub restart_count: u32,
    pub state: ContainerState,
    pub last_termination: Option<ContainerTermination>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerKind {
    Init,
    Application,
    Ephemeral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContainerState {
    Running {
        started_at: Option<DateTime<Utc>>,
    },
    Waiting {
        reason: Option<String>,
        message: Option<String>,
    },
    Terminated {
        termination: ContainerTermination,
    },
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerTermination {
    pub reason: Option<String>,
    pub message: Option<String>,
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}
