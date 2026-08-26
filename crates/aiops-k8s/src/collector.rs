use std::collections::{HashMap, HashSet};

use aiops_core::{
    observations::{
        CollectionError, HealthState, ObservationBundle, ObservationSource, ResourceEvent,
    },
    resources::{RelationshipKind, ResourceRef, ResourceRelationship},
};
use chrono::{DateTime, Duration, Utc};
use futures::{StreamExt, stream};
use k8s_openapi::{
    api::{
        apps::v1::{Deployment, ReplicaSet},
        core::v1::Pod,
    },
    apimachinery::pkg::apis::meta::v1::OwnerReference,
};
use kube::{ResourceExt, api::ObjectMeta};

use crate::{
    client::KubernetesClient,
    events,
    logs::{self, PodLogRequest},
    pods,
    workloads::{self, ParentDeployment},
};

#[derive(Debug, Clone)]
pub struct CollectionOptions {
    pub lookback: Duration,
    pub tail_lines: i64,
    pub include_logs: bool,
    pub include_previous_logs: bool,
    pub include_events: bool,
}

impl Default for CollectionOptions {
    fn default() -> Self {
        Self {
            lookback: Duration::minutes(15),
            tail_lines: 500,
            include_logs: true,
            include_previous_logs: true,
            include_events: true,
        }
    }
}

struct CollectedTopology {
    target: ResourceRef,
    deployment: Option<Deployment>,
    replica_sets: Vec<ReplicaSet>,
    pods: Vec<Pod>,
    errors: Vec<CollectionError>,
}

struct OwnedLogRequest {
    resource: ResourceRef,
    namespace: String,
    pod: String,
    container: String,
    previous: bool,
}

#[derive(Debug, Clone, Copy)]
enum PodLogSelection {
    All,
    UnhealthyOnly,
}

#[derive(Clone)]
pub struct KubernetesCollector {
    client: KubernetesClient,
    options: CollectionOptions,
}

impl KubernetesCollector {
    pub fn new(client: KubernetesClient, options: CollectionOptions) -> Self {
        Self { client, options }
    }

    pub async fn collect_pod(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<ObservationBundle, kube::Error> {
        let topology = self.resolve_pod(namespace, name).await?;
        Ok(self.collect_topology(topology, PodLogSelection::All).await)
    }

    pub async fn collect_replica_set(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<ObservationBundle, kube::Error> {
        let topology = self.resolve_replica_set(namespace, name).await?;
        Ok(self
            .collect_topology(topology, PodLogSelection::UnhealthyOnly)
            .await)
    }

    pub async fn collect_deployment(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<ObservationBundle, kube::Error> {
        let topology = self.resolve_deployment(namespace, name).await?;
        Ok(self
            .collect_topology(topology, PodLogSelection::UnhealthyOnly)
            .await)
    }

    async fn collect_topology(
        &self,
        topology: CollectedTopology,
        log_selection: PodLogSelection,
    ) -> ObservationBundle {
        let collection_started = Utc::now();

        let CollectedTopology {
            target,
            deployment,
            replica_sets,
            pods: collected_pods,
            errors,
        } = topology;

        let mut bundle = ObservationBundle {
            target,
            collected_from: collection_started - self.options.lookback,
            collected_at: collection_started,
            resources: Vec::new(),
            relationships: Vec::new(),
            workloads: Vec::new(),
            logs: Vec::new(),
            events: Vec::new(),
            health: Vec::new(),
            errors,
        };

        let deployment_ref = deployment.as_ref().map(|deployment| {
            let converted = workloads::convert_deployment(deployment);

            let resource = converted.snapshot.resource.clone();

            bundle.resources.push(converted.snapshot);
            bundle.workloads.push(converted.observation);

            resource
        });

        let mut replica_set_refs = HashMap::<String, ResourceRef>::new();

        for replica_set in &replica_sets {
            let converted = workloads::convert_replica_set(replica_set);
            let replica_set_ref = converted.snapshot.resource.clone();

            if let Some(uid) = replica_set.uid() {
                replica_set_refs.insert(uid, replica_set_ref.clone());
            }

            if let Some(deployment_ref) = &deployment_ref {
                let belongs_to_deployment =
                    controller_owner(&replica_set.metadata).is_some_and(|owner| {
                        owner.kind == "Deployment"
                            && owner.api_version == "apps/v1"
                            && deployment_ref.uid.as_deref() == Some(owner.uid.as_str())
                    });

                if belongs_to_deployment {
                    bundle.relationships.push(ResourceRelationship {
                        kind: RelationshipKind::ControllerOwner,
                        owner: deployment_ref.clone(),
                        dependent: replica_set_ref.clone(),
                    });
                }
            }

            bundle.resources.push(converted.snapshot);
            bundle.workloads.push(converted.observation);
        }

        for pod in &collected_pods {
            let snapshot = pods::snapshot(pod);
            let pod_ref = snapshot.resource.clone();
            let health = pods::health(pod);

            if let Some(replica_set_ref) = controller_owner(&pod.metadata)
                .filter(|owner| owner.kind == "ReplicaSet" && owner.api_version == "apps/v1")
                .and_then(|owner| replica_set_refs.get(&owner.uid))
            {
                bundle.relationships.push(ResourceRelationship {
                    kind: RelationshipKind::ControllerOwner,
                    owner: replica_set_ref.clone(),
                    dependent: pod_ref,
                });
            }

            bundle.resources.push(snapshot);
            bundle.health.push(health);
        }

        self.collect_topology_events(&mut bundle).await;
        deduplicate_events(&mut bundle.events);
        self.collect_topology_logs(&collected_pods, log_selection, &mut bundle)
            .await;

        bundle.events.sort_by(|left, right| {
            left.last_seen
                .cmp(&right.last_seen)
                .then_with(|| left.regarding.name.cmp(&right.regarding.name))
                .then_with(|| left.reason.cmp(&right.reason))
        });

        bundle.logs.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.resource.name.cmp(&right.resource.name))
                .then_with(|| left.container.cmp(&right.container))
        });

        bundle.collected_at = Utc::now();
        bundle
    }

    async fn collect_topology_events(&self, bundle: &mut ObservationBundle) {
        if !self.options.include_events {
            return;
        }

        let targets = bundle
            .resources
            .iter()
            .map(|snapshot| snapshot.resource.clone())
            .collect::<HashSet<_>>();

        for target in targets {
            match events::collect(self.client.inner(), &target).await {
                Ok(events) => {
                    bundle.events.extend(
                        events
                            .into_iter()
                            .filter(|event| event_within_window(event, bundle.collected_from)),
                    );
                }
                Err(error) => {
                    bundle.errors.push(CollectionError {
                        resource: Some(target.clone()),
                        source: ObservationSource::Events,
                        message: format!(
                            "failed to collect events for {:?} {}/{}: {error}",
                            target.kind,
                            target.namespace.as_deref().unwrap_or("<cluster>"),
                            target.name,
                        ),
                        retryable: is_retryable(&error),
                    });
                }
            }
        }
    }

    async fn collect_topology_logs(
        &self,
        collected_pods: &[Pod],
        log_selection: PodLogSelection,
        bundle: &mut ObservationBundle,
    ) {
        if !self.options.include_logs {
            return;
        }

        let requests =
            self.topology_log_requests(collected_pods, log_selection, &mut bundle.errors);

        let client = self.client.inner();
        let lookback = self.options.lookback.num_seconds();
        let tail_lines = self.options.tail_lines;

        let results = stream::iter(requests)
            .map(|request| {
                let client = client.clone();

                async move {
                    let result = logs::collect(
                        client,
                        request.resource.clone(),
                        PodLogRequest {
                            namespace: &request.namespace,
                            pod: &request.pod,
                            container: Some(&request.container),
                            since_seconds: Some(lookback),
                            tail_lines: Some(tail_lines),
                            previous: request.previous,
                        },
                    )
                    .await;

                    (request, result)
                }
            })
            .buffer_unordered(8)
            .collect::<Vec<_>>()
            .await;

        for (request, result) in results {
            match result {
                Ok(logs) => bundle.logs.extend(logs),
                Err(error) => {
                    let generation = if request.previous {
                        "previous"
                    } else {
                        "current"
                    };

                    bundle.errors.push(CollectionError {
                        resource: Some(request.resource),
                        source: ObservationSource::Logs,
                        message: format!(
                            "failed to collect {generation} logs for container {} in pod {}/{}: {error}",
                            request.container,
                            request.namespace,
                            request.pod
                        ),
                        retryable: is_retryable(&error),
                    });
                }
            }
        }
    }

    fn topology_log_requests(
        &self,
        collected_pods: &[Pod],
        log_selection: PodLogSelection,
        errors: &mut Vec<CollectionError>,
    ) -> Vec<OwnedLogRequest> {
        let mut requests = Vec::new();

        for pod in collected_pods {
            let health = pods::health(pod);

            let should_collect = match log_selection {
                PodLogSelection::All => true,
                PodLogSelection::UnhealthyOnly => {
                    health.state != HealthState::Healthy || health.restart_count.unwrap_or(0) > 0
                }
            };

            if !should_collect {
                continue;
            }

            let Some(namespace) = pod.metadata.namespace.clone() else {
                errors.push(missing_log_metadata_error(&health.resource, "namespace"));
                continue;
            };

            let Some(pod_name) = pod.metadata.name.clone() else {
                errors.push(missing_log_metadata_error(&health.resource, "name"));
                continue;
            };

            for container in health.containers {
                requests.push(OwnedLogRequest {
                    resource: health.resource.clone(),
                    namespace: namespace.clone(),
                    pod: pod_name.clone(),
                    container: container.name.clone(),
                    previous: false,
                });

                if self.options.include_previous_logs && container.restart_count > 0 {
                    requests.push(OwnedLogRequest {
                        resource: health.resource.clone(),
                        namespace: namespace.clone(),
                        pod: pod_name.clone(),
                        container: container.name,
                        previous: true,
                    });
                }
            }
        }

        requests
    }

    async fn resolve_pod(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<CollectedTopology, kube::Error> {
        let pod = pods::get(self.client.inner(), namespace, name).await?;
        let target = pods::snapshot(&pod).resource;

        let mut topology = CollectedTopology {
            target: target.clone(),
            deployment: None,
            replica_sets: Vec::new(),
            pods: vec![pod],
            errors: Vec::new(),
        };

        let pod = &topology.pods[0];

        let Some(replica_set_owner) = controller_owner(&pod.metadata)
            .filter(|owner| owner.kind == "ReplicaSet" && owner.api_version == "apps/v1")
        else {
            return Ok(topology);
        };

        let replica_set = match workloads::get_replica_set(
            self.client.inner(),
            namespace,
            &replica_set_owner.name,
        )
        .await
        {
            Ok(replica_set) => replica_set,
            Err(error) => {
                topology.errors.push(CollectionError {
                    resource: Some(target),
                    source: ObservationSource::ResourceState,
                    message: format!(
                        "failed to collect parent ReplicaSet {namespace}/{} for Pod {namespace}/{name}: {error}",
                        replica_set_owner.name,
                    ),
                    retryable: is_retryable(&error),
                });

                return Ok(topology);
            }
        };

        let actual_uid = replica_set.uid();

        if actual_uid.as_deref() != Some(replica_set_owner.uid.as_str()) {
            topology.errors.push(CollectionError {
                resource: Some(target),
                source: ObservationSource::ResourceState,
                message: format!(
                    "parent ReplicaSet {namespace}/{} has UID {}, but Pod {namespace}/{name} references UID {}; the ReplicaSet may have been deleted and recreated",
                    replica_set_owner.name,
                    actual_uid.as_deref().unwrap_or("<missing>"),
                    replica_set_owner.uid,
                ),
                retryable: false,
            });

            return Ok(topology);
        }

        let parent_deployment =
            workloads::deployment_for_replica_set(self.client.inner(), namespace, &replica_set)
                .await;

        match parent_deployment {
            Ok(ParentDeployment::Found(deployment)) => {
                topology.deployment = Some(deployment);
            }
            Ok(ParentDeployment::UidMismatch { expected, actual }) => {
                let replica_set_ref = workloads::replica_set_ref(&replica_set);

                topology.errors.push(CollectionError {
                    resource: Some(replica_set_ref),
                    source: ObservationSource::ResourceState,
                    message: format!(
                        "parent Deployment for ReplicaSet {namespace}/{} has UID {}, but its owner reference expects UID {expected}; the Deployment may have been deleted and recreated",
                        replica_set_owner.name,
                        actual.as_deref().unwrap_or("<missing>"),
                    ),
                    retryable: false,
                });
            }
            Ok(ParentDeployment::None) => {}
            Err(error) => {
                let replica_set_ref = workloads::replica_set_ref(&replica_set);

                topology.errors.push(CollectionError {
                    resource: Some(replica_set_ref),
                    source: ObservationSource::ResourceState,
                    message: format!(
                        "failed to collect parent Deployment for ReplicaSet {namespace}/{}: {error}",
                        replica_set_owner.name,
                    ),
                    retryable: is_retryable(&error),
                });
            }
        }

        topology.replica_sets.push(replica_set);

        Ok(topology)
    }

    async fn resolve_replica_set(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<CollectedTopology, kube::Error> {
        let replica_set = workloads::get_replica_set(self.client.inner(), namespace, name).await?;

        let target = workloads::replica_set_ref(&replica_set);

        let mut errors = Vec::new();

        let pods = match workloads::list_pods_for_replica_set(
            self.client.inner(),
            namespace,
            &replica_set,
        )
        .await
        {
            Ok(pods) => pods,
            Err(error) => {
                errors.push(CollectionError {
                    resource: Some(target.clone()),
                    source: ObservationSource::ResourceState,
                    message: format!(
                        "failed to list pods owned by ReplicaSet {namespace}/{name}: {error}"
                    ),
                    retryable: is_retryable(&error),
                });

                Vec::new()
            }
        };

        let deployment: Option<Deployment> = match workloads::deployment_for_replica_set(
            self.client.inner(),
            namespace,
            &replica_set,
        )
        .await
        {
            Ok(deployment) => match deployment {
                ParentDeployment::Found(deployment) => Some(deployment),
                ParentDeployment::UidMismatch { expected, actual } => {
                    errors.push(CollectionError {
                        resource: Some(target.clone()),
                        source: ObservationSource::ResourceState,
                        message: format!(
                            "parent Deployment for ReplicaSet {namespace}/{name} has UID {}, but the ReplicaSet owner reference expects UID {expected}; the Deployment may have been deleted and recreated",
                            actual.as_deref().unwrap_or("<missing>")
                        ),
                        retryable: false,
                    });

                    None
                }
                ParentDeployment::None => None,
            },
            Err(error) => {
                errors.push(CollectionError {
                    resource: Some(target.clone()),
                    source: ObservationSource::ResourceState,
                    message: format!("failed to collect parent Deployment for ReplicaSet {namespace}/{name}: {error}"),
                    retryable: is_retryable(&error),
                });

                None
            }
        };

        Ok(CollectedTopology {
            target,
            deployment,
            replica_sets: vec![replica_set],
            pods,
            errors,
        })
    }

    async fn resolve_deployment(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<CollectedTopology, kube::Error> {
        let deployment = workloads::get_deployment(self.client.inner(), namespace, name).await?;

        let target = workloads::deployment_ref(&deployment);

        let mut errors = Vec::new();

        let replica_sets = match workloads::list_replica_sets_for_deployment(
            self.client.inner(),
            namespace,
            &deployment,
        )
        .await
        {
            Ok(replica_sets) => replica_sets,
            Err(error) => {
                errors.push(CollectionError {
                    resource: Some(target.clone()),
                    source: ObservationSource::ResourceState,
                    message: format!(
                        "failed to list ReplicaSets owned by Deployment {namespace}/{name}: {error}"
                    ),
                    retryable: is_retryable(&error),
                });

                Vec::new()
            }
        };

        let owner_uids = replica_sets
            .iter()
            .filter_map(ResourceExt::uid)
            .collect::<HashSet<_>>();

        let pods = if owner_uids.is_empty() {
            Vec::new()
        } else {
            match workloads::list_pods_for_replica_sets(
                self.client.inner(),
                namespace,
                &deployment,
                &owner_uids,
            )
            .await
            {
                Ok(pods) => pods,
                Err(error) => {
                    errors.push(CollectionError {
                        resource: Some(target.clone()),
                        source: ObservationSource::ResourceState,
                        message: format!(
                            "failed to list pods owned by Deployment {namespace}/{name}: {error}"
                        ),
                        retryable: is_retryable(&error),
                    });

                    Vec::new()
                }
            }
        };

        Ok(CollectedTopology {
            target,
            deployment: Some(deployment),
            replica_sets,
            pods,
            errors,
        })
    }
}

fn controller_owner(metadata: &ObjectMeta) -> Option<&OwnerReference> {
    metadata
        .owner_references
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|owner| owner.controller == Some(true))
}

fn event_within_window(event: &ResourceEvent, cutoff: DateTime<Utc>) -> bool {
    event
        .last_seen
        .as_ref()
        .or(event.first_seen.as_ref())
        .is_none_or(|seen| *seen >= cutoff)
}

fn is_retryable(error: &kube::Error) -> bool {
    match error {
        kube::Error::Api(response) => {
            // 429 -> Too Many Requests
            // >= 500 -> Internal api server error
            response.code == 429 || response.code >= 500
        }
        _ => true,
    }
}

fn missing_log_metadata_error(resource: &ResourceRef, field: &str) -> CollectionError {
    let namespace = resource.namespace.as_deref().unwrap_or("<unknown>");

    let name = if resource.name.is_empty() {
        "<unknown>"
    } else {
        &resource.name
    };

    CollectionError {
        resource: Some(resource.clone()),
        source: ObservationSource::Logs,
        message: format!(
            "pod {namespace}/{name} has no metadata.{field}; logs cannot be collected"
        ),
        retryable: false,
    }
}

fn deduplicate_events(events: &mut Vec<ResourceEvent>) {
    let mut seen = HashSet::new();

    events.retain(|event| {
        seen.insert((
            event.regarding.clone(),
            event.reporting_controller.clone(),
            event.reason.clone(),
            event.message.clone(),
            event.first_seen,
            event.last_seen,
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiops_core::{observations::EventType, resources::ResourceKind};
    use chrono::TimeZone;

    fn event(first_seen: Option<DateTime<Utc>>, last_seen: Option<DateTime<Utc>>) -> ResourceEvent {
        ResourceEvent {
            regarding: ResourceRef {
                kind: ResourceKind::Pod,
                namespace: Some("default".into()),
                name: "api-0".into(),
                uid: None,
            },
            reporting_controller: None,
            event_type: EventType::Warning,
            reason: None,
            message: String::new(),
            first_seen,
            last_seen,
            count: 1,
        }
    }

    #[test]
    fn collection_options_have_bounded_defaults() {
        let options = CollectionOptions::default();

        assert_eq!(options.lookback, Duration::minutes(15));
        assert_eq!(options.tail_lines, 500);
        assert!(options.include_logs);
        assert!(options.include_previous_logs);
        assert!(options.include_events);
    }

    #[test]
    fn event_window_prefers_last_seen_and_includes_cutoff() {
        let cutoff = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let old = cutoff - Duration::minutes(1);

        assert!(event_within_window(&event(Some(old), Some(cutoff)), cutoff));
        assert!(!event_within_window(
            &event(Some(cutoff), Some(old)),
            cutoff
        ));
        assert!(event_within_window(&event(None, None), cutoff));
    }
}
