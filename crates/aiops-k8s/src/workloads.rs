use std::collections::HashSet;

use aiops_core::{
    observations::WorkloadObservation,
    resources::{
        ConditionStatus, ResourceCondition, ResourceKind, ResourceMetadata, ResourceRef,
        ResourceSnapshot,
    },
};
use chrono::{DateTime, Utc};
use k8s_openapi::api::{
    apps::v1::{Deployment, ReplicaSet},
    core::v1::Pod,
};
use kube::{
    Api, Client, ResourceExt,
    api::{ListParams, ObjectMeta},
    core::Selector,
};

use crate::convert::resource_ref;

pub enum ParentDeployment {
    None,
    Found(Box<Deployment>),
    UidMismatch {
        expected: String,
        actual: Option<String>,
    },
}

pub struct ConvertedWorkload {
    pub snapshot: ResourceSnapshot,
    pub observation: WorkloadObservation,
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

pub fn replica_set_ref(replica_set: &ReplicaSet) -> ResourceRef {
    resource_ref(replica_set, ResourceKind::ReplicaSet)
}

pub fn deployment_ref(deployment: &Deployment) -> ResourceRef {
    resource_ref(deployment, ResourceKind::Deployment)
}

pub fn convert_replica_set(replica_set: &ReplicaSet) -> ConvertedWorkload {
    let observed_at = Utc::now();
    let resource = replica_set_ref(replica_set);
    let status = replica_set.status.as_ref();

    let conditions = status
        .and_then(|status| status.conditions.as_deref())
        .unwrap_or_default()
        .iter()
        .map(|condition| {
            convert_condition(
                &condition.type_,
                &condition.status,
                condition.reason.as_deref(),
                condition.message.as_deref(),
            )
        })
        .collect::<Vec<_>>();

    let snapshot = convert_snapshot(
        &replica_set.metadata,
        resource.clone(),
        observed_at,
        status.and_then(|status| status.observed_generation),
        conditions.clone(),
    );

    let observation = WorkloadObservation {
        resource,
        observed_at,
        desired_replicas: desired_replicas(
            replica_set.spec.as_ref().and_then(|spec| spec.replicas),
        ),
        current_replicas: status
            .map(|status| non_negative(status.replicas))
            .unwrap_or(0),
        ready_replicas: status
            .and_then(|status| status.ready_replicas)
            .map(non_negative)
            .unwrap_or(0),
        available_replicas: status
            .and_then(|status| status.available_replicas)
            .map(non_negative),
        updated_replicas: None,
        conditions,
    };

    ConvertedWorkload {
        snapshot,
        observation,
    }
}

pub fn convert_deployment(deployment: &Deployment) -> ConvertedWorkload {
    let observed_at = Utc::now();
    let resource = deployment_ref(deployment);
    let status = deployment.status.as_ref();

    let conditions = status
        .and_then(|status| status.conditions.as_deref())
        .unwrap_or_default()
        .iter()
        .map(|condition| {
            convert_condition(
                &condition.type_,
                &condition.status,
                condition.reason.as_deref(),
                condition.message.as_deref(),
            )
        })
        .collect::<Vec<_>>();

    let snapshot = convert_snapshot(
        &deployment.metadata,
        resource.clone(),
        observed_at,
        status.and_then(|status| status.observed_generation),
        conditions.clone(),
    );

    let observation = WorkloadObservation {
        resource,
        observed_at,
        desired_replicas: desired_replicas(deployment.spec.as_ref().and_then(|spec| spec.replicas)),
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
        updated_replicas: status
            .and_then(|status| status.updated_replicas)
            .map(non_negative),
        conditions,
    };

    ConvertedWorkload {
        snapshot,
        observation,
    }
}

fn convert_condition(
    condition_type: &str,
    status: &str,
    reason: Option<&str>,
    message: Option<&str>,
) -> ResourceCondition {
    ResourceCondition {
        condition_type: condition_type.to_owned(),
        status: match status {
            "True" => ConditionStatus::True,
            "False" => ConditionStatus::False,
            _ => ConditionStatus::Unknown,
        },
        reason: reason.map(str::to_owned),
        message: message.map(str::to_owned),
    }
}

fn convert_snapshot(
    metadata: &ObjectMeta,
    resource: ResourceRef,
    observed_at: DateTime<Utc>,
    observed_generation: Option<i64>,
    conditions: Vec<ResourceCondition>,
) -> ResourceSnapshot {
    ResourceSnapshot {
        resource,
        observed_at,
        metadata: ResourceMetadata {
            labels: metadata.labels.clone().unwrap_or_default(),
            annotations: metadata.annotations.clone().unwrap_or_default(),
            generation: metadata.generation,
            observed_generation,
            deletion_timestamp: metadata.deletion_timestamp.as_ref().map(timestamp),
        },
        conditions,
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
        return Ok(ParentDeployment::UidMismatch {
            expected: owner.uid.clone(),
            actual: deployment.uid(),
        });
    }

    Ok(ParentDeployment::Found(Box::new(deployment)))
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
                        && owner.kind == "Deployment"
                        && owner.api_version == "apps/v1"
                        && owner.uid == deployment_uid
                })
        })
        .collect())
}

fn desired_replicas(value: Option<i32>) -> u32 {
    value.map(non_negative).unwrap_or(1)
}

fn non_negative(value: i32) -> u32 {
    value.max(0) as u32
}

fn timestamp(time: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Time) -> DateTime<Utc> {
    let secs = time.0.as_second();
    let nsecs = time.0.subsec_nanosecond() as u32;

    DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
}
