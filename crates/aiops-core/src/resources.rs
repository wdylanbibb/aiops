use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceRef {
    pub kind: ResourceKind,
    pub namespace: Option<String>,
    pub name: String,
    pub uid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceKind {
    Pod,
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
    Job,
    CronJob,
    Service,
    Ingress,
    ConfigMap,
    Secret,
    PersistentVolumeClaim,
    PersistentVolume,
    Node,
    Namespace,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub resource: ResourceRef,
    pub observed_at: DateTime<Utc>,
    pub metadata: ResourceMetadata,
    pub conditions: Vec<ResourceCondition>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceMetadata {
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub generation: Option<i64>,
    pub observed_generation: Option<i64>,
    pub deletion_timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceCondition {
    pub condition_type: String,
    pub status: ConditionStatus,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}
