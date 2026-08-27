use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use crate::observations::{
    events::{EventType, ResourceEvent},
    health::{
        ContainerHealth, ContainerKind, ContainerState, ContainerTermination, HealthObservation,
        HealthState,
    },
    logs::{LogEntry, LogStream},
    workloads::WorkloadObservation,
};
use crate::resources::{ResourceRef, ResourceRelationship, ResourceSnapshot};

mod events;
mod health;
mod logs;
mod workloads;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationBundle {
    pub target: ResourceRef,
    pub collected_from: DateTime<Utc>,
    pub collected_at: DateTime<Utc>,
    pub resources: Vec<ResourceSnapshot>,

    #[serde(default)]
    pub relationships: Vec<ResourceRelationship>,

    #[serde(default)]
    pub workloads: Vec<WorkloadObservation>,

    pub logs: Vec<LogEntry>,
    pub events: Vec<ResourceEvent>,
    pub health: Vec<HealthObservation>,
    pub errors: Vec<CollectionError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionError {
    pub resource: Option<ResourceRef>,
    pub source: ObservationSource,
    #[serde(default)]
    pub kind: CollectionErrorKind,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionErrorKind {
    OwnerLookup,
    ChildList,
    UidMismatch,
    EventList,
    LogRead,
    InvalidMetadata,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    ResourceState,
    Logs,
    Events,
    Health,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_deserialization_defaults_new_topology_fields_and_error_kind() {
        let bundle: ObservationBundle = serde_json::from_value(serde_json::json!({
            "target": {
                "kind": "pod",
                "namespace": "default",
                "name": "api-0",
                "uid": null
            },
            "collected_from": "2026-01-02T03:04:05Z",
            "collected_at": "2026-01-02T03:04:05Z",
            "resources": [],
            "logs": [],
            "events": [],
            "health": [],
            "errors": [{
                "resource": null,
                "source": "events",
                "message": "unavailable",
                "retryable": true
            }]
        }))
        .unwrap();

        assert!(bundle.relationships.is_empty());
        assert!(bundle.workloads.is_empty());
        assert_eq!(bundle.errors[0].kind, CollectionErrorKind::Unknown);
    }

    #[test]
    fn collection_error_contract_uses_stable_kind_names() {
        let error = CollectionError {
            resource: None,
            source: ObservationSource::ResourceState,
            kind: CollectionErrorKind::UidMismatch,
            message: "owner changed".into(),
            retryable: false,
        };

        let value = serde_json::to_value(error).unwrap();
        assert_eq!(value["source"], "resource_state");
        assert_eq!(value["kind"], "uid_mismatch");
    }
}
