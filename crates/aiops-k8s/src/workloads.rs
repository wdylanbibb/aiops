use aiops_core::{
    observations::WorkloadObservation,
    resources::{ResourceCondition, ResourceKind, ResourceMetadata, ResourceSnapshot},
};
use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use kube::{Api, Client, api::ListParams};

use crate::convert::resource_ref;

pub struct Workloads {
    pub deployments: Vec<Deployment>,
    pub stateful_sets: Vec<StatefulSet>,
    pub daemon_sets: Vec<DaemonSet>,
}

pub async fn list(client: Client, namespace: Option<&str>) -> Result<Workloads, kube::Error> {
    let deployments = match namespace {
        Some(namespace) => Api::<Deployment>::namespaced(client.clone(), namespace),
        None => Api::<Deployment>::all(client.clone()),
    };

    let stateful_sets = match namespace {
        Some(namespace) => Api::<StatefulSet>::namespaced(client.clone(), namespace),
        None => Api::<StatefulSet>::all(client.clone()),
    };

    let daemon_sets = match namespace {
        Some(namespace) => Api::<DaemonSet>::namespaced(client.clone(), namespace),
        None => Api::<DaemonSet>::all(client.clone()),
    };

    let params = ListParams::default();

    let (deployments, stateful_sets, daemon_sets) = tokio::try_join!(
        deployments.list(&params),
        stateful_sets.list(&params),
        daemon_sets.list(&params),
    )?;

    Ok(Workloads {
        deployments: deployments.items,
        stateful_sets: stateful_sets.items,
        daemon_sets: daemon_sets.items,
    })
}

pub async fn get_replica_set(
    client: Client,
    namespace: &str,
    name: &str,
) -> Result<ReplicaSet, kube::Error> {
    Api::<ReplicaSet>::namespaced(client, namespace)
        .get(name)
        .await
}

pub async fn get_deployment(
    client: Client,
    namespace: &str,
    name: &str,
) -> Result<Deployment, kube::Error> {
    Api::<Deployment>::namespaced(client, namespace)
        .get(name)
        .await
}

pub fn replica_set_snapshot(replica_set: &ReplicaSet) -> ResourceSnapshot {
    let metadata = &replica_set.metadata;

    ResourceSnapshot {
        resource: resource_ref(replica_set, ResourceKind::ReplicaSet),
        observed_at: Utc::now(),
        metadata: ResourceMetadata {
            labels: metadata.labels.clone().unwrap_or_default(),
            annotations: metadata.annotations.clone().unwrap_or_default(),
            generation: metadata.generation,
            observed_generation: replica_set
                .status
                .as_ref()
                .and_then(|status| status.observed_generation),
            deletion_timestamp: metadata.deletion_timestamp.as_ref().map(timestamp),
        },
        conditions: replica_set
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

pub fn deployment_snapshot(deployment: &Deployment) -> ResourceSnapshot {
    let metadata = &deployment.metadata;

    ResourceSnapshot {
        resource: resource_ref(deployment, ResourceKind::Deployment),
        observed_at: Utc::now(),
        metadata: ResourceMetadata {
            labels: metadata.labels.clone().unwrap_or_default(),
            annotations: metadata.annotations.clone().unwrap_or_default(),
            generation: metadata.generation,
            observed_generation: deployment
                .status
                .as_ref()
                .and_then(|status| status.observed_generation),
            deletion_timestamp: metadata.deletion_timestamp.as_ref().map(timestamp),
        },
        conditions: deployment
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

pub fn replica_set_observation(replica_set: &ReplicaSet) -> WorkloadObservation {
    let snapshot = replica_set_snapshot(replica_set);
    let status = replica_set.status.as_ref();

    WorkloadObservation {
        resource: snapshot.resource,
        observed_at: snapshot.observed_at,
        desired_replicas: replica_set
            .spec
            .as_ref()
            .and_then(|spec| spec.replicas)
            .map(non_negative)
            .unwrap_or(1),
        current_replicas: status.map(|status| status.replicas).map(non_negative).unwrap_or(0),
        ready_replicas: status
            .and_then(|status| status.ready_replicas)
            .map(non_negative)
            .unwrap_or(0),
        available_replicas: status
            .and_then(|status| status.available_replicas)
            .map(non_negative),
        conditions: snapshot.conditions,
    }
}

pub fn deployment_observation(deployment: &Deployment) -> WorkloadObservation {
    let snapshot = deployment_snapshot(deployment);
    let status = deployment.status.as_ref();

    WorkloadObservation {
        resource: snapshot.resource,
        observed_at: snapshot.observed_at,
        desired_replicas: deployment
            .spec
            .as_ref()
            .and_then(|spec| spec.replicas)
            .map(non_negative)
            .unwrap_or(1),
        current_replicas: status
            .and_then(|status| status.replicas)
            .map(non_negative)
            .unwrap_or(0),
        ready_replicas: status
            .and_then(|status| status.ready_replicas)
            .map(non_negative)
            .unwrap_or(0),
        available_replicas: status
            .and_then(|status| status.available_replicas)
            .map(non_negative),
        conditions: snapshot.conditions,
    }
}

fn non_negative(value: i32) -> u32 {
    value.max(0) as u32
}

fn timestamp(time: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Time) -> DateTime<Utc> {
    let secs = time.0.as_second();
    let nsecs = time.0.subsec_nanosecond() as u32;

    DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
}
