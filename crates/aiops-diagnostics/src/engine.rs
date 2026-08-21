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
