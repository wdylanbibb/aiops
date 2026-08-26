use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    observations::{ObservationBundle, ObservationSource},
    resources::ResourceRef,
};

pub trait DiagnosticRule: Send + Sync {
    fn evaluate(&self, bundle: &ObservationBundle) -> Vec<Finding>;
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub id: Uuid,
    pub status: IncidentStatus,
    pub severity: Severity,
    pub target: ResourceRef,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub observations: ObservationBundle,
    pub findings: Vec<Finding>,
}

impl Incident {
    pub fn new(observations: ObservationBundle, findings: Vec<Finding>) -> Self {
        let now = Utc::now();

        let severity = findings
            .iter()
            .map(|finding| finding.severity)
            .max()
            .unwrap_or(Severity::Info);

        let status = if !observations.errors.is_empty() {
            IncidentStatus::Incomplete
        } else if findings
            .iter()
            .any(|finding| matches!(finding.severity, Severity::Warning | Severity::Critical))
        {
            IncidentStatus::Open
        } else {
            IncidentStatus::Resolved
        };

        Self {
            id: Uuid::now_v7(),
            status,
            severity,
            target: observations.target.clone(),
            created_at: now,
            updated_at: now,
            observations,
            findings,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Open,
    Resolved,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub subject: ResourceRef,
    pub container: Option<String>,
    pub title: String,
    pub explanation: String,
    pub evidence: Vec<Evidence>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub source: ObservationSource,
    pub summary: String,
    pub timestamp: Option<DateTime<Utc>>,
}
