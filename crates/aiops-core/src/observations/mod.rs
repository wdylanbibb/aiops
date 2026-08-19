use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use crate::observations::{
    events::{EventType, ResourceEvent},
    health::{
        ContainerHealth, ContainerKind, ContainerState, ContainerTermination, HealthObservation,
        HealthState,
    },
    logs::{LogEntry, LogStream},
};
use crate::resources::{ResourceRef, ResourceSnapshot};

mod events;
mod health;
mod logs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationBundle {
    pub target: ResourceRef,
    pub collected_from: DateTime<Utc>,
    pub collected_at: DateTime<Utc>,
    pub resources: Vec<ResourceSnapshot>,
    pub logs: Vec<LogEntry>,
    pub events: Vec<ResourceEvent>,
    pub health: Vec<HealthObservation>,
    pub errors: Vec<CollectionError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionError {
    pub resource: Option<ResourceRef>,
    pub source: ObservationSource,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservationSource {
    ResourceState,
    Logs,
    Events,
    Health,
}
