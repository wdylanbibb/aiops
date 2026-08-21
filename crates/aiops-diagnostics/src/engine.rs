use aiops_core::{
    diagnostics::{Confidence, DiagnosisReport, DiagnosticRule, Finding, Severity},
    observations::ObservationBundle,
    resources::ResourceRef,
};
use chrono::Utc;
use std::{cmp::Ordering, collections::HashMap};

use crate::rules::{ContainerRestartRule, LogPatternRule, PodNotReadyRule, WarningEventRule};

pub struct DiagnosticEngine {
    rules: Vec<Box<dyn DiagnosticRule>>,
}

impl DiagnosticEngine {
    pub fn default_rules() -> Self {
        Self {
            rules: vec![
                Box::new(PodNotReadyRule),
                Box::new(ContainerRestartRule),
                Box::new(WarningEventRule),
                Box::new(LogPatternRule),
            ],
        }
    }

    pub fn diagnose(&self, bundle: &ObservationBundle) -> DiagnosisReport {
        let mut findings = self
            .rules
            .iter()
            .flat_map(|rule| rule.evaluate(bundle))
            .collect::<Vec<_>>();

        deduplicate_and_rank(&mut findings);

        DiagnosisReport {
            target: bundle.target.clone(),
            generated_at: Utc::now(),
            findings,
            incomplete: !bundle.errors.is_empty(),
            collection_errors: bundle.errors.clone(),
        }
    }
}

fn deduplicate_and_rank(findings: &mut Vec<Finding>) {
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct FindingKey {
        code: String,
        subject: ResourceRef,
        container: Option<String>,
    }

    let mut unique = HashMap::<FindingKey, Finding>::new();

    for finding in findings.drain(..) {
        let key = FindingKey {
            code: finding.code.clone(),
            subject: finding.subject.clone(),
            container: finding.container.clone(),
        };

        match unique.entry(key) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(finding);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                merge_finding(entry.get_mut(), finding);
            }
        }
    }

    findings.extend(unique.into_values());
    findings.sort_by(compare_findings);
}

fn merge_finding(existing: &mut Finding, incoming: Finding) {
    let incoming_is_stronger = finding_rank(&incoming) > finding_rank(existing);

    if severity_rank(&incoming.severity) > severity_rank(&existing.severity) {
        existing.severity = incoming.severity;
    }
    if confidence_rank(&incoming.confidence) > confidence_rank(&existing.confidence) {
        existing.confidence = incoming.confidence.clone();
    }

    if incoming_is_stronger {
        existing.title = incoming.title;
        existing.explanation = incoming.explanation;
    }

    for evidence in incoming.evidence {
        let duplicate = existing.evidence.iter().any(|candidate| {
            candidate.source == evidence.source
                && candidate.summary == evidence.summary
                && candidate.timestamp == evidence.timestamp
        });

        if !duplicate {
            existing.evidence.push(evidence);
        }
    }

    for recommendation in incoming.recommendations {
        if !existing.recommendations.contains(&recommendation) {
            existing.recommendations.push(recommendation);
        }
    }
}

fn finding_rank(finding: &Finding) -> (u8, u8) {
    (
        severity_rank(&finding.severity),
        confidence_rank(&finding.confidence),
    )
}

fn severity_rank(severity: &Severity) -> u8 {
    match severity {
        Severity::Info => 0,
        Severity::Warning => 1,
        Severity::Critical => 2,
    }
}

fn confidence_rank(confidence: &Confidence) -> u8 {
    match confidence {
        Confidence::Low => 0,
        Confidence::Medium => 1,
        Confidence::High => 2,
    }
}

fn compare_findings(left: &Finding, right: &Finding) -> Ordering {
    severity_rank(&right.severity)
        .cmp(&severity_rank(&left.severity))
        .then_with(|| confidence_rank(&right.confidence).cmp(&confidence_rank(&left.confidence)))
        .then_with(|| left.subject.namespace.cmp(&right.subject.namespace))
        .then_with(|| left.subject.name.cmp(&right.subject.name))
        .then_with(|| format!("{:?}", left.subject.kind).cmp(&format!("{:?}", right.subject.kind)))
        .then_with(|| left.container.cmp(&right.container))
        .then_with(|| left.code.cmp(&right.code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiops_core::{
        diagnostics::Evidence,
        observations::{CollectionError, ObservationSource},
        resources::ResourceKind,
    };
    use chrono::TimeZone;

    fn resource(name: &str) -> ResourceRef {
        ResourceRef {
            kind: ResourceKind::Pod,
            namespace: Some("default".into()),
            name: name.into(),
            uid: None,
        }
    }

    fn finding(name: &str, code: &str, severity: Severity, confidence: Confidence) -> Finding {
        Finding {
            code: code.into(),
            severity,
            confidence,
            subject: resource(name),
            container: None,
            title: format!("{code} title"),
            explanation: format!("{code} explanation"),
            evidence: vec![],
            recommendations: vec![],
        }
    }

    #[test]
    fn deduplication_merges_unique_details_and_keeps_strongest_content() {
        let timestamp = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut weak = finding("api-0", "same", Severity::Warning, Confidence::Low);
        weak.evidence.push(Evidence {
            source: ObservationSource::Logs,
            summary: "first".into(),
            timestamp: Some(timestamp),
        });
        weak.recommendations.push("inspect logs".into());
        let mut strong = finding("api-0", "same", Severity::Critical, Confidence::High);
        strong.title = "strong title".into();
        strong.evidence.push(Evidence {
            source: ObservationSource::Logs,
            summary: "first".into(),
            timestamp: Some(timestamp),
        });
        strong.evidence.push(Evidence {
            source: ObservationSource::Events,
            summary: "second".into(),
            timestamp: None,
        });
        strong
            .recommendations
            .extend(["inspect logs".into(), "restart safely".into()]);
        let mut findings = vec![weak, strong];

        deduplicate_and_rank(&mut findings);

        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].severity, Severity::Critical));
        assert!(matches!(findings[0].confidence, Confidence::High));
        assert_eq!(findings[0].title, "strong title");
        assert_eq!(findings[0].evidence.len(), 2);
        assert_eq!(findings[0].recommendations.len(), 2);
    }

    #[test]
    fn ranking_is_severity_then_confidence_then_identity() {
        let mut findings = vec![
            finding("z", "warning-low", Severity::Warning, Confidence::Low),
            finding("b", "critical-low", Severity::Critical, Confidence::Low),
            finding("a", "critical-high", Severity::Critical, Confidence::High),
        ];
        deduplicate_and_rank(&mut findings);
        assert_eq!(
            findings
                .iter()
                .map(|item| item.code.as_str())
                .collect::<Vec<_>>(),
            ["critical-high", "critical-low", "warning-low"]
        );
    }

    #[test]
    fn report_is_marked_incomplete_when_collection_had_errors() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let input = ObservationBundle {
            target: resource("api-0"),
            collected_from: now,
            collected_at: now,
            resources: vec![],
            logs: vec![],
            events: vec![],
            health: vec![],
            errors: vec![CollectionError {
                resource: None,
                source: ObservationSource::Events,
                message: "unavailable".into(),
                retryable: true,
            }],
        };
        let report = DiagnosticEngine::default_rules().diagnose(&input);
        assert!(report.incomplete);
        assert_eq!(report.collection_errors.len(), 1);
        assert_eq!(report.target, input.target);
    }
}
