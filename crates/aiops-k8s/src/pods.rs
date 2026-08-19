use aiops_core::{
    observations::{
        ContainerHealth, ContainerKind, ContainerState as HealthContainerState,
        ContainerTermination, HealthObservation, HealthState,
    },
    resources::{ResourceCondition, ResourceKind, ResourceMetadata, ResourceSnapshot},
};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{
    ContainerState as KubeContainerState, ContainerStateTerminated, ContainerStatus, Pod,
};
use kube::{Api, Client, api::ListParams};

use crate::convert::resource_ref;

pub async fn list(client: Client, namespace: Option<&str>) -> Result<Vec<Pod>, kube::Error> {
    let pods = match namespace {
        Some(ns) => Api::<Pod>::namespaced(client, ns),
        None => Api::<Pod>::all(client),
    };

    Ok(pods.list(&ListParams::default()).await?.items)
}

pub async fn get(client: Client, namespace: &str, name: &str) -> Result<Pod, kube::Error> {
    Api::<Pod>::namespaced(client, namespace).get(name).await
}

pub fn snapshot(pod: &Pod) -> ResourceSnapshot {
    let metadata = &pod.metadata;

    ResourceSnapshot {
        resource: resource_ref(pod, ResourceKind::Pod),
        observed_at: Utc::now(),
        metadata: ResourceMetadata {
            labels: metadata.labels.clone().unwrap_or_default(),
            annotations: metadata.annotations.clone().unwrap_or_default(),
            generation: metadata.generation,
            observed_generation: None,
            deletion_timestamp: metadata.deletion_timestamp.as_ref().map(timestamp),
        },
        conditions: pod
            .status
            .as_ref()
            .and_then(|status| status.conditions.as_ref())
            .into_iter()
            .flatten()
            .map(|condition| ResourceCondition {
                condition_type: condition.type_.clone(),
                status: match condition.status.as_str() {
                    "True" => aiops_core::resources::ConditionStatus::True,
                    "False" => aiops_core::resources::ConditionStatus::False,
                    _ => aiops_core::resources::ConditionStatus::Unknown,
                },
                reason: condition.reason.clone(),
                message: condition.message.clone(),
            })
            .collect(),
    }
}

pub fn health(pod: &Pod) -> HealthObservation {
    let containers = container_health_observations(pod);
    let state = overall_health_state(pod, &containers);
    let status = pod.status.as_ref();

    let restart_count = status
        .and_then(|status| status.container_statuses.as_ref())
        .map(|containers| {
            containers
                .iter()
                .map(|container| container.restart_count.max(0) as u32)
                .sum()
        });

    let started_at = status
        .and_then(|status| status.start_time.as_ref())
        .map(timestamp);

    let ready_since = status
        .and_then(|status| status.conditions.as_ref())
        .into_iter()
        .flatten()
        .find(|condition| condition.type_ == "Ready" && condition.status == "True")
        .and_then(|condition| condition.last_transition_time.as_ref().map(timestamp));

    HealthObservation {
        resource: resource_ref(pod, ResourceKind::Pod),
        observed_at: Utc::now(),
        state,
        started_at,
        ready_since,
        restart_count,
        conditions: snapshot(pod).conditions,
        containers,
    }
}

fn overall_health_state(pod: &Pod, containers: &[ContainerHealth]) -> HealthState {
    let status = pod.status.as_ref();

    let ready = status
        .and_then(|status| status.conditions.as_ref())
        .into_iter()
        .flatten()
        .find(|condition| condition.type_ == "Ready")
        .map(|condition| condition.status == "True");

    let phase = status.and_then(|status| status.phase.as_deref());

    if pod.metadata.deletion_timestamp.is_some() {
        HealthState::Terminating
    } else if phase == Some("Failed") || has_unhealthy_container(containers) {
        HealthState::Unhealthy
    } else {
        match (phase, ready) {
            (Some("Running"), Some(true)) => HealthState::Healthy,
            (Some("Running"), _) => HealthState::Degraded,
            (Some("Pending"), _) => HealthState::Pending,
            (Some("Succeeded"), _) => HealthState::Healthy,
            _ => HealthState::Unknown,
        }
    }
}

fn container_health_observations(pod: &Pod) -> Vec<ContainerHealth> {
    let status = pod.status.as_ref();

    status
        .into_iter()
        .flat_map(|status| {
            let application = status
                .container_statuses
                .iter()
                .flatten()
                .map(|container| container_health(container, ContainerKind::Application));

            let init = status
                .init_container_statuses
                .iter()
                .flatten()
                .map(|container| container_health(container, ContainerKind::Init));

            let ephemeral = status
                .ephemeral_container_statuses
                .iter()
                .flatten()
                .map(|container| container_health(container, ContainerKind::Ephemeral));

            application.chain(init).chain(ephemeral)
        })
        .collect::<Vec<_>>()
}

fn has_unhealthy_container(containers: &[ContainerHealth]) -> bool {
    containers.iter().any(|container| {
        matches!(
            &container.state,
                HealthContainerState::Waiting {
                    reason: Some(reason),
                    ..
                } if matches!(
                reason.as_str(),
                    "CrashLoopBackOff"
                        | "ImagePullBackOff"
                        | "ErrImagePull"
                        | "CreateContainerConfigError"
                        | "CreateContainerError"
            )
        ) || matches!(
            &container.state,
                HealthContainerState::Terminated { termination }
                if termination.exit_code != 0
        )
    })
}

fn container_health(status: &ContainerStatus, kind: ContainerKind) -> ContainerHealth {
    let last_termination = status
        .last_state
        .as_ref()
        .and_then(|state| state.terminated.as_ref())
        .map(container_termination);

    ContainerHealth {
        name: status.name.clone(),
        kind,
        ready: status.ready,
        restart_count: status.restart_count.max(0) as u32,
        state: container_state(status.state.as_ref()),
        last_termination,
    }
}

fn container_state(state: Option<&KubeContainerState>) -> HealthContainerState {
    let Some(state) = state else {
        return HealthContainerState::Unknown;
    };

    if let Some(running) = &state.running {
        return HealthContainerState::Running {
            started_at: running.started_at.as_ref().map(timestamp),
        };
    };

    if let Some(waiting) = &state.waiting {
        return HealthContainerState::Waiting {
            reason: waiting.reason.clone(),
            message: waiting.message.clone(),
        };
    };

    if let Some(terminated) = &state.terminated {
        return HealthContainerState::Terminated {
            termination: container_termination(terminated),
        };
    };

    HealthContainerState::Unknown
}

fn container_termination(terminated: &ContainerStateTerminated) -> ContainerTermination {
    ContainerTermination {
        reason: terminated.reason.clone(),
        message: terminated.message.clone(),
        exit_code: terminated.exit_code,
        signal: terminated.signal,
        started_at: terminated.started_at.as_ref().map(timestamp),
        finished_at: terminated.finished_at.as_ref().map(timestamp),
    }
}

fn timestamp(time: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Time) -> DateTime<Utc> {
    let secs = time.0.as_second();
    let nsecs = time.0.subsec_nanosecond() as u32;

    DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
}
