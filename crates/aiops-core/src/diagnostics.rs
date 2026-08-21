use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    observations::{CollectionError, ObservationBundle, ObservationSource},
    resources::ResourceRef,
};

pub trait DiagnosticRule: Send + Sync {
    fn evaluate(&self, bundle: &ObservationBundle) -> Vec<Finding>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticFinding {
    pub id: String,
    pub category: FindingCategory,
    pub title: String,
    pub explanation: String,
    pub confidence: Confidence,
    pub subject: ResourceRef,
    pub evidence_ids: Vec<String>,
    pub contributing_resources: Vec<ResourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisReport {
    pub target: ResourceRef,
    pub generated_at: DateTime<Utc>,
    pub findings: Vec<Finding>,
    pub incomplete: bool,
    pub collection_errors: Vec<CollectionError>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindingCategory {
    CrashLoop,
    ImagePull,
    Scheduling,
    Readiness,
    ResourceExhaustion,
    Storage,
    Networking,
    Configuration,
    Rollout,
    Dependency,
    Unknown,
}
