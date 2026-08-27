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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        observations::{CollectionError, CollectionErrorKind},
        resources::ResourceKind,
    };
    use chrono::TimeZone;

    fn target() -> ResourceRef {
        ResourceRef {
            kind: ResourceKind::Pod,
            namespace: Some("default".into()),
            name: "api-0".into(),
            uid: Some("pod-uid".into()),
        }
    }

    fn bundle() -> ObservationBundle {
        let at = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        ObservationBundle {
            target: target(),
            collected_from: at,
            collected_at: at,
            resources: vec![],
            relationships: vec![],
            workloads: vec![],
            logs: vec![],
            events: vec![],
            health: vec![],
            errors: vec![],
        }
    }

    fn finding(severity: Severity) -> Finding {
        Finding {
            code: "test.finding".into(),
            severity,
            confidence: Confidence::High,
            subject: target(),
            container: None,
            title: "test finding".into(),
            explanation: "test explanation".into(),
            evidence: vec![],
            recommendations: vec![],
        }
    }

    #[test]
    fn incident_derives_target_severity_status_and_timestamps() {
        let observations = bundle();
        let incident = Incident::new(
            observations.clone(),
            vec![finding(Severity::Warning), finding(Severity::Critical)],
        );

        assert_eq!(incident.target, observations.target);
        assert_eq!(incident.status, IncidentStatus::Open);
        assert_eq!(incident.severity, Severity::Critical);
        assert_eq!(incident.created_at, incident.updated_at);
        assert_eq!(incident.findings.len(), 2);
    }

    #[test]
    fn incident_is_resolved_without_actionable_findings() {
        let incident = Incident::new(bundle(), vec![finding(Severity::Info)]);

        assert_eq!(incident.status, IncidentStatus::Resolved);
        assert_eq!(incident.severity, Severity::Info);
    }

    #[test]
    fn collection_errors_make_incident_incomplete_even_with_findings() {
        let mut observations = bundle();
        observations.errors.push(CollectionError {
            resource: Some(target()),
            source: ObservationSource::Events,
            kind: CollectionErrorKind::EventList,
            message: "events unavailable".into(),
            retryable: true,
        });

        let incident = Incident::new(observations, vec![finding(Severity::Critical)]);

        assert_eq!(incident.status, IncidentStatus::Incomplete);
        assert_eq!(incident.severity, Severity::Critical);
    }

    #[test]
    fn incident_contract_serializes_enums_as_snake_case() {
        let incident = Incident::new(bundle(), vec![finding(Severity::Warning)]);
        let value = serde_json::to_value(incident).unwrap();

        assert_eq!(value["status"], "open");
        assert_eq!(value["severity"], "warning");
        assert_eq!(value["target"]["kind"], "pod");
        assert_eq!(value["findings"][0]["confidence"], "high");
    }
}
