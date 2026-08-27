use std::collections::{HashMap, HashSet};

use aiops_core::{
    diagnostics::{Confidence, DiagnosticRule, Evidence, Finding, Severity},
    observations::{
        ContainerHealth, ContainerState, ContainerTermination, EventType, HealthState, LogEntry,
        ObservationBundle, ObservationSource, WorkloadObservation,
    },
    resources::{ConditionStatus, RelationshipKind, ResourceKind, ResourceRef},
};
use chrono::{DateTime, Utc};

pub struct PodNotReadyRule;

impl DiagnosticRule for PodNotReadyRule {
    fn evaluate(
        &self,
        bundle: &aiops_core::observations::ObservationBundle,
    ) -> Vec<aiops_core::diagnostics::Finding> {
        bundle.health.iter().filter_map(|health| {
            fn condition_explanation(condition: &aiops_core::resources::ResourceCondition) -> String {
                match (&condition.reason, &condition.message) {
                    (Some(reason), Some(message)) => format!("Ready condition is {:?}: {reason}: {message}", condition.status),
                    (Some(reason), None) => format!("Ready condition is {:?}: {reason}", condition.status),
                    (None, Some(message)) => format!("Ready condition is {:?}: {message}", condition.status),
                    (None, None) => format!("Ready condition is {:?}", condition.status)
                }
            }

            let severity = match health.state {
                HealthState::Unhealthy => Severity::Critical,
                HealthState::Degraded | HealthState::Pending => Severity::Warning,
                HealthState::Healthy | HealthState::Terminating | HealthState::Unknown => return None,
            };

            let ready_condition = health.conditions.iter().find(|condition| condition.condition_type == "Ready");

            let confidence = match ready_condition {
                Some(condition) if condition.status == ConditionStatus::False => {
                    Confidence::High
                }
                _ => Confidence::Medium,
            };

            let explanation = ready_condition.map(condition_explanation).unwrap_or_else(|| format!("Pod health was reported as {:?}, but no Ready condition was available.", health.state));

            let mut evidence = vec![Evidence {
                source: ObservationSource::Health,
                summary: format!("Pod health state is {:?}", health.state),
                timestamp: Some(health.observed_at),
            }];

            if let Some(condition) = ready_condition {
                evidence.push(Evidence {
                    source: ObservationSource::Health,
                    summary: condition_explanation(condition),
                    timestamp: Some(health.observed_at),
                });
            }

            let mut recommendations = vec!["Inspect the pod's container states and recent warning events.".to_owned()];

            match ready_condition.and_then(|condition| condition.reason.as_deref()) {
                Some("ContainersNotReady") => recommendations.push("Inspect logs and termination details for the unready containers.".to_owned()),
                Some("PodScheduled") | Some("Unschedulable") => recommendations.push("Inspect scheduling events, resource requests, taints, and node capacity.".to_owned()),
                _ => recommendations.push("Inspect readiness probe results and container logs.".to_owned())
            }

            Some(Finding {
                code: "pod.not_ready".to_owned(),
                severity,
                confidence,
                subject: health.resource.clone(),
                container: None,
                title: format!("Pod {} is not ready", health.resource.name),
                explanation,
                evidence,
                recommendations
            })
        }).collect()
    }
}

pub struct ContainerRestartRule;

impl DiagnosticRule for ContainerRestartRule {
    fn evaluate(
        &self,
        bundle: &aiops_core::observations::ObservationBundle,
    ) -> Vec<aiops_core::diagnostics::Finding> {
        bundle
            .health
            .iter()
            .flat_map(|health| {
                health.containers.iter().filter_map(move |container| {
                    evaluate_container(container, &health.resource, health.observed_at)
                })
            })
            .collect()
    }
}

fn evaluate_container(
    container: &ContainerHealth,
    resource: &ResourceRef,
    observed_at: DateTime<Utc>,
) -> Option<Finding> {
    if container.restart_count == 0 {
        return None;
    }

    let termination = effective_termination(container);
    let waiting_reason = match &container.state {
        ContainerState::Waiting { reason, .. } => reason.as_deref(),
        _ => None,
    };

    let diagnosis = if waiting_reason == Some("CrashLoopBackOff") {
        RestartDiagnosis {
            code: "container.restart.crash_loop",
            severity: Severity::Critical,
            confidence: Confidence::High,
            title: format!("Container {} is crash looping", container.name),
            explanation: format!(
                "Container {} has restarted {} time(s) and is waiting with reason CrashLoopBackOff.",
                container.name, container.restart_count
            ),
        }
    } else if termination.and_then(|value| value.reason.as_deref()) == Some("OOMKilled") {
        RestartDiagnosis {
            code: "container.restart.oom_killed",
            severity: Severity::Critical,
            confidence: Confidence::High,
            title: format!(
                "Container {} was killed for exceeding memory",
                container.name
            ),
            explanation: format!(
                "Container {} has restarted {} time(s), and its previous execution was terminated with reason OOMKilled.",
                container.name, container.restart_count
            ),
        }
    } else if termination.is_some_and(abnormal_termination) {
        let termination = termination.expect("termination was checked above");

        RestartDiagnosis {
            code: "container.restart.failed",
            severity: Severity::Warning,
            confidence: Confidence::High,
            title: format!("Container {} restarted after failure", container.name),
            explanation: format!(
                "Container {} has restarted {} time(s). Its previous execution exited with code {}{}.",
                container.name,
                container.restart_count,
                termination.exit_code,
                termination
                    .reason
                    .as_deref()
                    .map(|reason| format!(" ({reason})"))
                    .unwrap_or_default(),
            ),
        }
    } else {
        RestartDiagnosis {
            code: "container.restart.detected",
            severity: Severity::Warning,
            confidence: Confidence::Medium,
            title: format!("Container {} has restarted", container.name),
            explanation: format!(
                "Container {} has restarted {} time(s), but no conclusive failure reason was available.",
                container.name, container.restart_count
            ),
        }
    };

    let mut evidence = vec![Evidence {
        source: ObservationSource::Health,
        summary: format!(
            "Container {} restart count is {}",
            container.name, container.restart_count
        ),
        timestamp: Some(observed_at),
    }];

    if let Some(reason) = waiting_reason {
        evidence.push(Evidence {
            source: ObservationSource::Health,
            summary: format!("Container {} is waiting: {reason}", container.name),
            timestamp: Some(observed_at),
        });
    }

    if let Some(termination) = termination {
        evidence.push(Evidence {
            source: ObservationSource::Health,
            summary: termination_summary(&container.name, termination),
            timestamp: termination
                .finished_at
                .as_ref()
                .cloned()
                .or(Some(observed_at)),
        });
    }

    Some(Finding {
        code: diagnosis.code.to_owned(),
        severity: diagnosis.severity,
        confidence: diagnosis.confidence,
        subject: resource.clone(),
        container: Some(container.name.clone()),
        title: diagnosis.title,
        explanation: diagnosis.explanation,
        evidence,
        recommendations: restart_recommendations(waiting_reason, termination),
    })
}

struct RestartDiagnosis {
    code: &'static str,
    severity: Severity,
    confidence: Confidence,
    title: String,
    explanation: String,
}

fn effective_termination(container: &ContainerHealth) -> Option<&ContainerTermination> {
    container.last_termination.as_ref().or({
        if let ContainerState::Terminated { termination } = &container.state {
            Some(termination)
        } else {
            None
        }
    })
}

fn abnormal_termination(termination: &ContainerTermination) -> bool {
    termination.exit_code != 0 || termination.signal.is_some()
}

fn termination_summary(name: &str, termination: &ContainerTermination) -> String {
    let reason = termination.reason.as_deref().unwrap_or("unknown reason");
    let signal = termination
        .signal
        .map(|signal| format!(", signal {signal}"))
        .unwrap_or_default();

    format!(
        "Container {name} previously terminated with reason {reason}, exit_code {}{signal}",
        termination.exit_code
    )
}

fn restart_recommendations(
    waiting_reason: Option<&str>,
    termination: Option<&ContainerTermination>,
) -> Vec<String> {
    let mut recommendations = vec![
        "Inspect the container's previous logs near the termination time.".to_owned(),
        "Verify the container command, configuration, and required dependencies.".to_owned(),
    ];

    if waiting_reason == Some("CrashLoopBackOff") {
        recommendations
            .push("Inspect startup, liveness, and readiness probe configuration.".to_owned());
    }

    if termination.and_then(|value| value.reason.as_deref()) == Some("OOMKilled") {
        recommendations
            .push("Compare observed memory usage with the container's memory limit.".to_owned());
        recommendations
            .push("Increase the memory limit or reduce the application's memory usage.".to_owned());
    }

    recommendations
}

pub struct WarningEventRule;

impl DiagnosticRule for WarningEventRule {
    fn evaluate(
        &self,
        bundle: &aiops_core::observations::ObservationBundle,
    ) -> Vec<aiops_core::diagnostics::Finding> {
        bundle
            .events
            .iter()
            .filter(|event| event.event_type == EventType::Warning)
            .map(|event| Finding {
                code: format!(
                    "kubernetes.event.{}",
                    event.reason.as_deref().unwrap_or("unknown").to_lowercase()
                ),
                severity: Severity::Warning,
                confidence: Confidence::High,
                subject: event.regarding.clone(),
                container: None,
                title: event
                    .reason
                    .clone()
                    .unwrap_or_else(|| "Kubernetes warning".to_owned()),
                explanation: event.message.clone(),
                evidence: vec![Evidence {
                    source: ObservationSource::Events,
                    summary: event.message.clone(),
                    timestamp: event.last_seen,
                }],
                recommendations: recommendations_for(event.reason.as_deref()),
            })
            .collect()
    }
}

fn recommendations_for(reason: Option<&str>) -> Vec<String> {
    let recommendations: &[&str] = match reason {
        Some("FailedScheduling") | Some("Unschedulable") => &[
            "Inspect the event message for insufficient resources, node selectors, affinity rules, and untolerated taints.",
            "Compare the pod's CPU and memory requests with allocatable capacity on eligible nodes.",
            "Verify that any required persistent volume claims are bound.",
        ],
        Some("FailedPull") | Some("ErrImagePull") | Some("ImagePullBackOff") => &[
            "Verify the container image name and tag and confirm that the image exists in the registry.",
            "Check image pull secrets and the workload's service account for registry credentials.",
            "Confirm that cluster nodes can resolve and connect to the image registry.",
        ],
        Some("FailedMount") | Some("FailedAttachVolume") | Some("FailedMapVolume") => &[
            "Inspect the referenced persistent volume claims, volumes, ConfigMaps, and Secrets.",
            "Verify storage class provisioning, access modes, node attachment limits, and CSI driver health.",
        ],
        Some("Unhealthy") => &[
            "Inspect the event message to identify whether the readiness, liveness, or startup probe failed.",
            "Verify the probe path, port, timeout, and initial delay against the application's startup behavior.",
            "Inspect container logs around the probe failures.",
        ],
        Some("BackOff") => &[
            "Inspect the container's current and previous logs and its last termination reason and exit code.",
            "Verify the container command, arguments, configuration, and required dependencies.",
        ],
        Some("FailedCreate")
        | Some("CreateContainerConfigError")
        | Some("CreateContainerError") => &[
            "Inspect the event message for a missing ConfigMap, Secret, service account, or invalid container configuration.",
            "Verify the workload's security context, command, environment variables, and volume references.",
        ],
        Some("FailedCreatePodSandBox") | Some("NetworkNotReady") => &[
            "Inspect the node's container runtime and network plugin status.",
            "Check CNI plugin logs, node conditions, and available pod IP capacity.",
        ],
        Some("Evicted") => &[
            "Inspect node pressure conditions and the eviction message to identify the exhausted resource.",
            "Review the pod's resource requests and limits and the affected node's disk, memory, and inode usage.",
        ],
        Some("NodeNotReady") => &[
            "Inspect the node's Ready condition, kubelet status, and recent node events.",
            "Verify node networking, disk availability, and connectivity to the Kubernetes API server.",
        ],
        _ => &[
            "Inspect the complete Kubernetes event message and other recent events for the affected resource.",
            "Inspect the resource status and related pod logs for additional failure details.",
        ],
    };

    recommendations
        .iter()
        .map(|recommendation| (*recommendation).to_owned())
        .collect()
}

pub struct LogPatternRule;

impl DiagnosticRule for LogPatternRule {
    fn evaluate(
        &self,
        bundle: &aiops_core::observations::ObservationBundle,
    ) -> Vec<aiops_core::diagnostics::Finding> {
        let mut groups = HashMap::<PatternKey, PatternGroup>::new();

        for log in &bundle.logs {
            let Some(pattern) = match_pattern(&log.message) else {
                continue;
            };

            let key = PatternKey {
                namespace: log.resource.namespace.clone(),
                resource: log.resource.clone(),
                container: log.container.clone(),
                code: pattern.code,
            };

            let group = groups.entry(key).or_insert_with(|| PatternGroup {
                pattern,
                match_count: 0,
                evidence: Vec::new(),
            });

            group.match_count += 1;

            if group.evidence.len() < 5 {
                group.evidence.push(log_evidence(log));
            }
        }

        groups
            .into_iter()
            .map(|(key, group)| Finding {
                code: group.pattern.code.to_owned(),
                severity: group.pattern.severity,
                confidence: group.pattern.confidence,
                subject: key.resource,
                container: key.container,
                title: group.pattern.title.to_owned(),
                explanation: format!(
                    "Detected {} log line(s) matching {}.",
                    group.match_count, group.pattern.description,
                ),
                evidence: group.evidence,
                recommendations: group
                    .pattern
                    .recommendations
                    .iter()
                    .map(|recommendation| (*recommendation).to_owned())
                    .collect(),
            })
            .collect()
    }
}

pub struct WorkloadReplicaAvailabilityRule;

impl DiagnosticRule for WorkloadReplicaAvailabilityRule {
    fn evaluate(&self, bundle: &aiops_core::observations::ObservationBundle) -> Vec<Finding> {
        bundle
            .workloads
            .iter()
            .filter(|workload| {
                workload.desired_replicas > 0 && workload.ready_replicas < workload.desired_replicas
            })
            .map(|workload| {
                let severity = if workload.ready_replicas == 0 {
                    Severity::Critical
                } else {
                    Severity::Warning
                };

                Finding {
                    code: "workload.replicas_unavailable".into(),
                    severity,
                    confidence: Confidence::High,
                    subject: workload.resource.clone(),
                    container: None,
                    title: format!("{} has unavailable replicas", workload.resource.name),
                    explanation: format!(
                        "{} of {} desired replicas are ready.",
                        workload.ready_replicas, workload.desired_replicas,
                    ),
                    evidence: vec![replica_evidence(workload)],
                    recommendations: vec![
                        "Inspect the health and events of the workload's owned pods.".into(),
                        "Inspect rollout conditions and recent workload changes.".into(),
                    ],
                }
            })
            .collect()
    }
}

fn replica_evidence(workload: &WorkloadObservation) -> Evidence {
    Evidence {
        source: ObservationSource::ResourceState,
        summary: format!(
            "desired={}, current={}, ready={}, available={}, updated={}",
            workload.desired_replicas,
            workload.current_replicas,
            workload.ready_replicas,
            workload
                .available_replicas
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            workload
                .updated_replicas
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
        ),
        timestamp: Some(workload.observed_at),
    }
}

pub struct WorkloadRolloutStalledRule;

impl DiagnosticRule for WorkloadRolloutStalledRule {
    fn evaluate(&self, bundle: &aiops_core::observations::ObservationBundle) -> Vec<Finding> {
        bundle
            .workloads
            .iter()
            .filter(|workload| workload.resource.kind == ResourceKind::Deployment)
            .filter_map(|workload| {
                let condition = workload.conditions.iter().find(|condition| {
                    condition.condition_type == "Progressing"
                        && condition.status == ConditionStatus::False
                        && condition.reason.as_deref() == Some("ProgressDeadlineExceeded")
                })?;

                let detail = condition
                    .message
                    .as_deref()
                    .or(condition.reason.as_deref())
                    .unwrap_or("the rollout exceeded its progress deadline");

                Some(Finding {
                    code: "workload.rollout_stalled".into(),
                    severity: Severity::Critical,
                    confidence: Confidence::High,
                    subject: workload.resource.clone(),
                    container: None,
                    title: format!("Deployment {} rollout is stalled", workload.resource.name),
                    explanation: detail.to_owned(),
                    evidence: vec![Evidence {
                        source: ObservationSource::ResourceState,
                        summary: format!(
                            "Progressing=False, reason={}",
                            condition.reason.as_deref().unwrap_or("unknown"),
                        ),
                        timestamp: Some(workload.observed_at),
                    }],
                    recommendations: vec![
                        "Inspect unavailable pods and their container states.".into(),
                        "Review the Deployment's rollout history and latest template changes."
                            .into(),
                        "Verify image, scheduling, probe, and configuration events.".into(),
                    ],
                })
            })
            .collect()
    }
}

pub struct WorkloadGenerationRule;

impl DiagnosticRule for WorkloadGenerationRule {
    fn evaluate(&self, bundle: &aiops_core::observations::ObservationBundle) -> Vec<Finding> {
        bundle
            .resources
            .iter()
            .filter(|snapshot| {
                matches!(
                &snapshot.resource.kind,
                    ResourceKind::Deployment | ResourceKind::ReplicaSet
            )
            })
            .filter_map(|snapshot| {
                let generation = snapshot.metadata.generation?;
                let observed = snapshot.metadata.observed_generation?;

                if observed >= generation {
                    return None;
                }

                Some(Finding {
                    code: "workload.generation_unobserved".into(),
                    severity: Severity::Warning,
                    confidence: Confidence::Medium,
                    subject: snapshot.resource.clone(),
                    container: None,
                    title: format!(
                        "{} has not observed its latest generation",
                        snapshot.resource.name
                    ),
                    explanation: format!(
                        "Resource generation is {generation}, but the controller has observed generation {observed}.",
                    ),
                    evidence: vec![Evidence {
                        source: ObservationSource::ResourceState,
                        summary: format!(
                            "generation={generation}, observed_generation={observed}",
                        ),
                        timestamp: Some(snapshot.observed_at),
                    }],
                    recommendations: vec![
                        "Check the controller manager and recent resource events.".into(),
                        "Verify that the workload specification is valid and can be reconciled.".into()
                    ],
                })
            })
            .collect()
    }
}

pub struct WorkloadNoOwnedPodsRule;

impl DiagnosticRule for WorkloadNoOwnedPodsRule {
    fn evaluate(&self, bundle: &aiops_core::observations::ObservationBundle) -> Vec<Finding> {
        // Avoid claiming there are no pods when topology collection failed
        if bundle.errors.iter().any(|error| {
            matches!(
                error.kind,
                aiops_core::observations::CollectionErrorKind::ChildList
                    | aiops_core::observations::CollectionErrorKind::InvalidMetadata
            )
        }) {
            return Vec::new();
        }

        bundle
            .workloads
            .iter()
            .filter(|workload| workload.desired_replicas > 0)
            .filter(|workload| !has_reachable_pod(bundle, &workload.resource))
            .map(|workload| Finding {
                code: "workload.no_owned_pods".into(),
                severity: Severity::Critical,
                confidence: Confidence::High,
                subject: workload.resource.clone(),
                container: None,
                title: format!(
                    "{} has no owned pods",
                    workload.resource.name,
                ),
                explanation: format!(
                    "The workload desires {} replica(s), but no owned pods were found.",
                    workload.desired_replicas,
                ),
                evidence: vec![Evidence {
                    source: ObservationSource::ResourceState,
                    summary: format!(
                        "desired_replicas={}, reachable_pods=0",
                        workload.desired_replicas,
                    ),
                    timestamp: Some(workload.observed_at),
                }],
                recommendations: vec![
                    "Inspect workload and ReplicaSet events for pod creation failures.".into(),
                    "Verify quotas, admission policies, service accounts, and pod template configuration.".into(),
                ],
            })
            .collect()
    }
}

fn has_reachable_pod(bundle: &ObservationBundle, root: &ResourceRef) -> bool {
    let mut pending = vec![root.clone()];
    let mut visited = HashSet::new();

    while let Some(resource) = pending.pop() {
        if !visited.insert(resource.clone()) {
            continue;
        }

        for relationship in bundle.relationships.iter().filter(|relationship| {
            relationship.kind == RelationshipKind::ControllerOwner && relationship.owner == resource
        }) {
            if relationship.dependent.kind == ResourceKind::Pod {
                return true;
            }

            pending.push(relationship.dependent.clone());
        }
    }

    false
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PatternKey {
    namespace: Option<String>,
    resource: ResourceRef,
    container: Option<String>,
    code: &'static str,
}

struct PatternGroup {
    pattern: MatchedPattern,
    match_count: usize,
    evidence: Vec<Evidence>,
}

struct MatchedPattern {
    code: &'static str,
    title: &'static str,
    description: &'static str,
    severity: Severity,
    confidence: Confidence,
    recommendations: &'static [&'static str],
}

fn match_pattern(message: &str) -> Option<MatchedPattern> {
    fn contains_any(message: &str, patterns: &[&str]) -> bool {
        patterns.iter().any(|pattern| message.contains(pattern))
    }

    let normalized = message.to_ascii_lowercase();

    if contains_any(
        &normalized,
        &[
            "out of memory",
            "cannot allocate memory",
            "java.lang.outofmemoryerror",
            "heap out of memory",
        ],
    ) {
        return Some(MatchedPattern {
            code: "log.memory_exhausted",
            title: "Possible memory exhaustion",
            description: "a memory-exhaustion signature",
            severity: Severity::Critical,
            confidence: Confidence::Medium,
            recommendations: &[
                "Compare memory usage with the container's memory request and limit.",
                "Inspect the container's termination reason for OOMKilled.",
                "Check for memory leaks, oversized workloads, and unbounded caches.",
            ],
        });
    }

    if contains_any(
        &normalized,
        &[
            "panicked at",
            "panic:",
            "fatal error",
            "segmentation fault",
            "segfault",
        ],
    ) {
        return Some(MatchedPattern {
            code: "log.process_crash",
            title: "Application crash signature",
            description: "a panic or process-crash signature",
            severity: Severity::Critical,
            confidence: Confidence::High,
            recommendations: &[
                "Inspect the surrounding current and previous container logs.",
                "Correlate the failure with the container's exit code and termination reason.",
                "Review recent application, configuration, and dependency changes.",
            ],
        });
    }

    if contains_any(
        &normalized,
        &[
            "connection refused",
            "connection reset by peer",
            "no route to host",
            "network is unreachable",
            "i/o timeout",
        ],
    ) {
        return Some(MatchedPattern {
            code: "log.connection_failure",
            title: "Dependency connection failure",
            description: "a network or dependency connection failure",
            severity: Severity::Warning,
            confidence: Confidence::Medium,
            recommendations: &[
                "Identify the destination host and port from the surrounding logs.",
                "Verify the destination Service, endpoints, and backing pods.",
                "Check NetworkPolicies, DNS resolution, and dependency health.",
            ],
        });
    }

    if contains_any(
        &normalized,
        &[
            "certificate has expired",
            "certificate verify failed",
            "unknown certificate authority",
            "tls handshake error",
            "x509:",
        ],
    ) {
        return Some(MatchedPattern {
            code: "log.tls_failure",
            title: "TLS or certificate failure",
            description: "a TLS handshake or certificate validation failure",
            severity: Severity::Warning,
            confidence: Confidence::Medium,
            recommendations: &[
                "Inspect certificate validity, trust chains, and configured server names.",
                "Verify mounted certificate Secrets and certificate rotation.",
                "Check clock synchronization on the workload and node.",
            ],
        });
    }

    None
}

fn log_evidence(log: &LogEntry) -> Evidence {
    let container = log.container.as_deref().unwrap_or("unknown container");
    let generation = if log.previous_container {
        "previous"
    } else {
        "current"
    };

    Evidence {
        source: ObservationSource::Logs,
        summary: format!("{container} ({generation}): {}", excerpt(&log.message, 300)),
        timestamp: log.timestamp.as_ref().cloned(),
    }
}

fn excerpt(message: &str, maximum_chars: usize) -> String {
    let flattened = message.replace(['\r', '\n'], " ");
    let mut characters = flattened.chars();

    let excerpt: String = characters.by_ref().take(maximum_chars).collect();

    if characters.next().is_some() {
        format!("{excerpt}…")
    } else {
        excerpt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiops_core::{
        diagnostics::DiagnosticRule,
        observations::{
            CollectionError, CollectionErrorKind, ContainerKind, HealthObservation, LogStream,
            ObservationBundle, ResourceEvent,
        },
        resources::{
            ResourceCondition, ResourceKind, ResourceMetadata, ResourceRelationship,
            ResourceSnapshot,
        },
    };
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap()
    }

    fn resource(name: &str) -> ResourceRef {
        ResourceRef {
            kind: ResourceKind::Pod,
            namespace: Some("default".into()),
            name: name.into(),
            uid: None,
        }
    }

    fn workload_resource(kind: ResourceKind, name: &str) -> ResourceRef {
        ResourceRef {
            kind,
            namespace: Some("default".into()),
            name: name.into(),
            uid: Some(format!("{name}-uid")),
        }
    }

    fn workload(kind: ResourceKind, name: &str, desired: u32, ready: u32) -> WorkloadObservation {
        WorkloadObservation {
            resource: workload_resource(kind, name),
            observed_at: at(),
            desired_replicas: desired,
            current_replicas: ready,
            ready_replicas: ready,
            available_replicas: Some(ready),
            updated_replicas: Some(ready),
            conditions: vec![],
        }
    }

    fn bundle() -> ObservationBundle {
        ObservationBundle {
            target: resource("api-0"),
            collected_from: at(),
            collected_at: at(),
            resources: vec![],
            relationships: vec![],
            workloads: vec![],
            logs: vec![],
            events: vec![],
            health: vec![],
            errors: Vec::<CollectionError>::new(),
        }
    }

    fn container(name: &str, restart_count: u32, state: ContainerState) -> ContainerHealth {
        ContainerHealth {
            name: name.into(),
            kind: ContainerKind::Application,
            ready: false,
            restart_count,
            state,
            last_termination: None,
        }
    }

    #[test]
    fn pod_not_ready_uses_condition_for_confidence_and_guidance() {
        let mut input = bundle();
        input.health.push(HealthObservation {
            resource: resource("api-0"),
            observed_at: at(),
            state: HealthState::Unhealthy,
            started_at: None,
            ready_since: None,
            restart_count: None,
            conditions: vec![ResourceCondition {
                condition_type: "Ready".into(),
                status: ConditionStatus::False,
                reason: Some("ContainersNotReady".into()),
                message: Some("api is unready".into()),
            }],
            containers: vec![],
        });

        let findings = PodNotReadyRule.evaluate(&input);
        assert_eq!(findings.len(), 1);
        assert!(matches!(findings[0].severity, Severity::Critical));
        assert!(matches!(findings[0].confidence, Confidence::High));
        assert!(findings[0].explanation.contains("api is unready"));
        assert!(
            findings[0]
                .recommendations
                .iter()
                .any(|item| item.contains("termination details"))
        );
    }

    #[test]
    fn pod_not_ready_ignores_non_actionable_health_states() {
        let mut input = bundle();
        for state in [
            HealthState::Healthy,
            HealthState::Terminating,
            HealthState::Unknown,
        ] {
            input.health.push(HealthObservation {
                resource: resource("api-0"),
                observed_at: at(),
                state,
                started_at: None,
                ready_since: None,
                restart_count: None,
                conditions: vec![],
                containers: vec![],
            });
        }
        assert!(PodNotReadyRule.evaluate(&input).is_empty());
    }

    #[test]
    fn restart_rule_prioritizes_crash_loop_over_oom_termination() {
        let mut input = bundle();
        let mut crashing = container(
            "api",
            4,
            ContainerState::Waiting {
                reason: Some("CrashLoopBackOff".into()),
                message: None,
            },
        );
        crashing.last_termination = Some(ContainerTermination {
            reason: Some("OOMKilled".into()),
            message: None,
            exit_code: 137,
            signal: None,
            started_at: None,
            finished_at: Some(at()),
        });
        input.health.push(HealthObservation {
            resource: resource("api-0"),
            observed_at: at(),
            state: HealthState::Unhealthy,
            started_at: None,
            ready_since: None,
            restart_count: Some(4),
            conditions: vec![],
            containers: vec![crashing],
        });

        let findings = ContainerRestartRule.evaluate(&input);
        assert_eq!(findings[0].code, "container.restart.crash_loop");
        assert!(matches!(findings[0].severity, Severity::Critical));
        assert_eq!(findings[0].evidence.len(), 3);
        assert!(
            findings[0]
                .recommendations
                .iter()
                .any(|item| item.contains("memory limit"))
        );
    }

    #[test]
    fn restart_rule_ignores_containers_that_never_restarted() {
        let mut input = bundle();
        input.health.push(HealthObservation {
            resource: resource("api-0"),
            observed_at: at(),
            state: HealthState::Healthy,
            started_at: None,
            ready_since: None,
            restart_count: Some(0),
            conditions: vec![],
            containers: vec![container("api", 0, ContainerState::Unknown)],
        });
        assert!(ContainerRestartRule.evaluate(&input).is_empty());
    }

    #[test]
    fn warning_event_rule_filters_normal_events_and_specializes_advice() {
        let mut input = bundle();
        for event_type in [EventType::Normal, EventType::Warning] {
            input.events.push(ResourceEvent {
                regarding: resource("api-0"),
                reporting_controller: None,
                event_type,
                reason: Some("FailedScheduling".into()),
                message: "0/3 nodes available".into(),
                first_seen: None,
                last_seen: Some(at()),
                count: 2,
            });
        }

        let findings = WarningEventRule.evaluate(&input);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "kubernetes.event.failedscheduling");
        assert_eq!(findings[0].recommendations.len(), 3);
    }

    #[test]
    fn log_patterns_are_case_insensitive_grouped_and_evidence_is_capped() {
        let mut input = bundle();
        for index in 0..7 {
            input.logs.push(LogEntry {
                resource: resource("api-0"),
                timestamp: Some(at()),
                container: Some("api".into()),
                stream: LogStream::Stderr,
                message: format!("PANIC: failure number {index}"),
                previous_container: index == 0,
            });
        }
        input.logs.push(LogEntry {
            resource: resource("api-0"),
            timestamp: None,
            container: Some("worker".into()),
            stream: LogStream::Stderr,
            message: "connection refused".into(),
            previous_container: false,
        });

        let mut findings = LogPatternRule.evaluate(&input);
        findings.sort_by(|left, right| left.code.cmp(&right.code));
        assert_eq!(findings.len(), 2);
        let crash = findings
            .iter()
            .find(|finding| finding.code == "log.process_crash")
            .unwrap();
        assert_eq!(crash.evidence.len(), 5);
        assert!(crash.explanation.contains("7 log line(s)"));
        assert!(crash.evidence[0].summary.contains("(previous)"));
    }

    #[test]
    fn replica_availability_severity_tracks_outage_and_ignores_scale_to_zero() {
        let mut input = bundle();
        input.workloads.extend([
            workload(ResourceKind::Deployment, "down", 3, 0),
            workload(ResourceKind::ReplicaSet, "degraded", 3, 2),
            workload(ResourceKind::Deployment, "scaled-down", 0, 0),
        ]);

        let findings = WorkloadReplicaAvailabilityRule.evaluate(&input);

        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[1].severity, Severity::Warning);
        assert!(findings[0].evidence[0].summary.contains("updated=0"));
    }

    #[test]
    fn rollout_stalled_requires_the_deployment_deadline_condition() {
        let mut input = bundle();
        let mut deployment = workload(ResourceKind::Deployment, "api", 3, 1);
        deployment.conditions.push(ResourceCondition {
            condition_type: "Progressing".into(),
            status: ConditionStatus::False,
            reason: Some("ProgressDeadlineExceeded".into()),
            message: Some("deployment exceeded its progress deadline".into()),
        });
        let mut replica_set = deployment.clone();
        replica_set.resource = workload_resource(ResourceKind::ReplicaSet, "api-abc");
        input.workloads.extend([deployment, replica_set]);

        let findings = WorkloadRolloutStalledRule.evaluate(&input);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject.kind, ResourceKind::Deployment);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert!(findings[0].explanation.contains("progress deadline"));
    }

    #[test]
    fn generation_rule_only_reports_controller_lag() {
        let mut input = bundle();
        for (name, generation, observed_generation) in [("lagging", 4, 3), ("current", 4, 4)] {
            input.resources.push(ResourceSnapshot {
                resource: workload_resource(ResourceKind::Deployment, name),
                observed_at: at(),
                metadata: ResourceMetadata {
                    generation: Some(generation),
                    observed_generation: Some(observed_generation),
                    ..Default::default()
                },
                conditions: vec![],
            });
        }

        let findings = WorkloadGenerationRule.evaluate(&input);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject.name, "lagging");
    }

    #[test]
    fn owned_pod_rule_traverses_deployment_replica_set_pod_graph() {
        let mut input = bundle();
        let deployment = workload(ResourceKind::Deployment, "api", 2, 0);
        let replica_set = workload_resource(ResourceKind::ReplicaSet, "api-abc");
        let pod = resource("api-abc-123");
        input.workloads.push(deployment.clone());
        input.relationships.extend([
            ResourceRelationship {
                kind: RelationshipKind::ControllerOwner,
                owner: deployment.resource,
                dependent: replica_set.clone(),
            },
            ResourceRelationship {
                kind: RelationshipKind::ControllerOwner,
                owner: replica_set,
                dependent: pod,
            },
        ]);

        assert!(WorkloadNoOwnedPodsRule.evaluate(&input).is_empty());
    }

    #[test]
    fn owned_pod_rule_reports_absence_but_suppresses_incomplete_child_lists() {
        let mut input = bundle();
        input
            .workloads
            .push(workload(ResourceKind::Deployment, "api", 2, 0));

        let findings = WorkloadNoOwnedPodsRule.evaluate(&input);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "workload.no_owned_pods");

        input.errors.push(CollectionError {
            resource: None,
            source: ObservationSource::ResourceState,
            kind: CollectionErrorKind::ChildList,
            message: "replica sets could not be listed".into(),
            retryable: true,
        });
        assert!(WorkloadNoOwnedPodsRule.evaluate(&input).is_empty());
    }

    #[test]
    fn excerpt_flattens_unicode_safely_and_marks_truncation() {
        assert_eq!(excerpt("a\nb\r\nc", 20), "a b  c");
        assert_eq!(excerpt("éclair", 2), "éc…");
    }
}
