use aiops_core::{
    observations::{HealthObservation, HealthState},
    resources::{ResourceCondition, ResourceKind, ResourceMetadata, ResourceSnapshot},
};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Pod;
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
            deletion_timestamp: metadata.deletion_timestamp.as_ref().map(|time| {
                let secs = time.0.as_second();
                let nsecs = time.0.subsec_nanosecond() as u32;

                DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
            }),
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
    let status = pod.status.as_ref();

    let restart_count = status
        .and_then(|status| status.container_statuses.as_ref())
        .map(|containers| {
            containers
                .iter()
                .map(|container| container.restart_count.max(0) as u32)
                .sum()
        });

    let ready = status
        .and_then(|status| status.conditions.as_ref())
        .into_iter()
        .flatten()
        .find(|condition| condition.type_ == "Ready")
        .map(|condition| condition.status == "True");

    let state = match (status.and_then(|status| status.phase.as_deref()), ready) {
        (Some("Running"), Some(true)) => HealthState::Healthy,
        (Some("Running"), _) => HealthState::Degraded,
        (Some("Pending"), _) => HealthState::Pending,
        (Some("Failed"), _) => HealthState::Unhealthy,
        (Some("Succeeded"), _) => HealthState::Healthy,
        _ => HealthState::Unknown,
    };

    let started_at = status
        .and_then(|status| status.start_time.as_ref())
        .map(|time| {
            let secs = time.0.as_second();
            let nsecs = time.0.subsec_nanosecond() as u32;

            DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
        });

    let ready_since = status
        .and_then(|status| status.conditions.as_ref())
        .into_iter()
        .flatten()
        .find(|condition| condition.type_ == "Ready" && condition.status == "True")
        .and_then(|condition| {
            condition.last_transition_time.as_ref().map(|time| {
                let secs = time.0.as_second();
                let nsecs = time.0.subsec_nanosecond() as u32;

                DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
            })
        });

    HealthObservation {
        resource: resource_ref(pod, ResourceKind::Pod),
        observed_at: Utc::now(),
        state,
        started_at,
        ready_since,
        restart_count,
        conditions: snapshot(pod).conditions,
    }
}
