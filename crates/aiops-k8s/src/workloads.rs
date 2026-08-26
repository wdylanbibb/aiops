use std::collections::HashSet;

use aiops_core::{
    observations::WorkloadObservation,
    resources::{ResourceCondition, ResourceKind, ResourceMetadata, ResourceSnapshot},
};
use chrono::{DateTime, Utc};
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet},
    core::v1::Pod,
};
use kube::{Api, Client, ResourceExt, api::ListParams, core::Selector};

use crate::convert::resource_ref;

pub struct Workloads {
    pub deployments: Vec<Deployment>,
    pub stateful_sets: Vec<StatefulSet>,
    pub daemon_sets: Vec<DaemonSet>,
}

pub enum ParentDeployment {
    None,
    Found(Deployment),
    UidMismatch {
        expected: String,
        actual: Option<String>,
    }
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
        current_replicas: status
            .map(|status| status.replicas)
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

pub async fn deployment_for_replica_set(
    client: Client,
    namespace: &str,
    replica_set: &ReplicaSet,
) -> Result<ParentDeployment, kube::Error> {
    let Some(owner) = replica_set
        .metadata
        .owner_references
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|owner| {
            owner.controller == Some(true)
                && owner.kind == "Deployment"
                && owner.api_version == "apps/v1"
        })
    else {
        return Ok(ParentDeployment::None);
    };

    let deployment = get_deployment(client, namespace, &owner.name).await?;

    if deployment.uid().as_deref() != Some(owner.uid.as_str()) {
        return Ok(ParentDeployment::UidMismatch { expected: owner.uid.clone(), actual: deployment.uid() });
    }

    Ok(ParentDeployment::Found(deployment))
}

pub async fn list_pods_for_replica_set(
    client: Client,
    namespace: &str,
    replica_set: &ReplicaSet,
) -> Result<Vec<Pod>, kube::Error> {
    let Some(replica_set_uid) = replica_set.uid() else {
        return Ok(Vec::new());
    };

    let params = replica_set
        .spec
        .as_ref()
        .and_then(|spec| Selector::try_from(spec.selector.clone()).ok())
        .map(|selector| ListParams::default().labels_from(&selector))
        .unwrap_or_default();

    let listed = Api::<Pod>::namespaced(client, namespace)
        .list(&params)
        .await?;

    Ok(listed
        .items
        .into_iter()
        .filter(|pod| {
            pod.metadata
                .owner_references
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|owner| {
                    owner.controller == Some(true)
                        && owner.kind == "ReplicaSet"
                        && owner.api_version == "apps/v1"
                        && owner.uid == replica_set_uid
                })
        })
        .collect())
}

pub async fn list_pods_for_replica_sets(
    client: Client,
    namespace: &str,
    deployment: &Deployment,
    owner_uids: &HashSet<String>,
) -> Result<Vec<Pod>, kube::Error> {
    if owner_uids.is_empty() {
        return Ok(Vec::new());
    }

    let params = deployment
        .spec
        .as_ref()
        .and_then(|spec| Selector::try_from(spec.selector.clone()).ok())
        .map(|selector| ListParams::default().labels_from(&selector))
        .unwrap_or_default();

    let listed = Api::<Pod>::namespaced(client, namespace)
        .list(&params)
        .await?;

    Ok(listed
        .items
        .into_iter()
        .filter(|pod| {
            pod.metadata
                .owner_references
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|owner| {
                    owner.controller == Some(true)
                        && owner.kind == "ReplicaSet"
                        && owner.api_version == "apps/v1"
                        && owner_uids.contains(&owner.uid)
                })
        })
        .collect())
}

pub async fn list_replica_sets_for_deployment(
    client: Client,
    namespace: &str,
    deployment: &Deployment,
) -> Result<Vec<ReplicaSet>, kube::Error> {
    let Some(deployment_uid) = deployment.uid() else {
        return Ok(Vec::new());
    };

    let params = deployment
        .spec
        .as_ref()
        .and_then(|spec| Selector::try_from(spec.selector.clone()).ok())
        .map(|selector| ListParams::default().labels_from(&selector))
        .unwrap_or_default();

    let listed = Api::<ReplicaSet>::namespaced(client, namespace)
        .list(&params)
        .await?;

    Ok(listed
        .items
        .into_iter()
        .filter(|replica_set| {
            replica_set
                .metadata
                .owner_references
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|owner| {
                    owner.controller == Some(true)
                        && owner.api_version == "apps/v1"
                        && owner.uid == deployment_uid
                })
        })
        .collect())
}

fn non_negative(value: i32) -> u32 {
    value.max(0) as u32
}

fn timestamp(time: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Time) -> DateTime<Utc> {
    let secs = time.0.as_second();
    let nsecs = time.0.subsec_nanosecond() as u32;

    DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
}
