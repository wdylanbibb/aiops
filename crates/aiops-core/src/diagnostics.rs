use serde::{Deserialize, Serialize};

use crate::resources::ResourceRef;

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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence(f32);

impl Confidence {
    pub fn new(value: f32) -> Option<Self> {
        (0.0..=1.0).contains(&value).then_some(Self(value))
    }

    pub fn value(self) -> f32 {
        self.0
    }
}
