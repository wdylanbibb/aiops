use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::resources::{ResourceCondition, ResourceRef};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadObservation {
    pub resource: ResourceRef,
    pub observed_at: DateTime<Utc>,
    pub desired_replicas: u32,
    pub current_replicas: u32,
    pub ready_replicas: u32,
    pub available_replicas: Option<u32>,

    #[serde(default)]
    pub updated_replicas: Option<u32>,

    pub conditions: Vec<ResourceCondition>,
}
