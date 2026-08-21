use aiops_core::{
    observations::{
        CollectionError, ContainerHealth, ObservationBundle, ObservationSource, ResourceEvent,
    },
    resources::ResourceRef,
};
use chrono::{DateTime, Duration, Utc};
use k8s_openapi::api::core::v1::Pod;

use crate::{
    client::KubernetesClient,
    events,
    logs::{self, PodLogRequest},
    pods,
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
        let collected_at = Utc::now();
        let pod = pods::get(self.client.inner(), namespace, name).await?;

        let snapshot = pods::snapshot(&pod);
        let target = snapshot.resource.clone();

        let health = pods::health(&pod);
        let containers = health.containers.clone();

        let mut bundle = ObservationBundle {
            target: target.clone(),
            collected_from: collected_at - self.options.lookback,
            collected_at,
            resources: vec![snapshot],
            logs: Vec::new(),
            events: Vec::new(),
            health: vec![health],
            errors: Vec::new(),
        };

        if self.options.include_events {
            match events::collect(self.client.inner(), &target).await {
                Ok(events) => {
                    bundle.events = events
                        .into_iter()
                        .filter(|event| event_within_window(event, bundle.collected_from))
                        .collect();
                }
                Err(error) => bundle.errors.push(CollectionError {
                    resource: Some(target.clone()),
                    source: ObservationSource::Events,
                    message: error.to_string(),
                    retryable: is_retryable(&error),
                }),
            }
        }

        if self.options.include_logs {
            self.collect_container_logs(&pod, &target, &containers, &mut bundle)
                .await;
        }

        Ok(bundle)
    }

    async fn collect_container_logs(
        &self,
        pod: &Pod,
        target: &ResourceRef,
        containers: &[ContainerHealth],
        bundle: &mut ObservationBundle,
    ) {
        let Some(namespace) = pod.metadata.namespace.as_deref() else {
            bundle.errors.push(CollectionError {
                resource: Some(target.clone()),
                source: ObservationSource::Logs,
                message: "pod has no namespace; logs cannot be collected".to_owned(),
                retryable: false,
            });
            return;
        };

        let Some(pod_name) = pod.metadata.name.as_deref() else {
            bundle.errors.push(CollectionError {
                resource: Some(target.clone()),
                source: ObservationSource::Logs,
                message: "pod has no name; logs cannot be collected".to_owned(),
                retryable: false,
            });
            return;
        };

        for container in containers {
            self.collect_log_stream(namespace, pod_name, &container.name, false, target, bundle)
                .await;

            if self.options.include_previous_logs && container.restart_count > 0 {
                self.collect_log_stream(namespace, pod_name, &container.name, true, target, bundle)
                    .await;
            }
        }
    }

    async fn collect_log_stream(
        &self,
        namespace: &str,
        pod_name: &str,
        container: &str,
        previous: bool,
        target: &ResourceRef,
        bundle: &mut ObservationBundle,
    ) {
        let request = PodLogRequest {
            namespace,
            pod: pod_name,
            container: Some(container),
            since_seconds: Some(self.options.lookback.num_seconds()),
            tail_lines: Some(self.options.tail_lines),
            previous,
        };

        match logs::collect(self.client.inner(), target.clone(), request).await {
            Ok(logs) => bundle.logs.extend(logs),
            Err(error) => {
                let generation = if previous { "previous" } else { "current" };

                bundle.errors.push(CollectionError {
                    resource: Some(target.clone()),
                    source: ObservationSource::Logs,
                    message: format!(
                        "failed to collect {generation} logs for container {container}: {error}"
                    ),
                    retryable: is_retryable(&error),
                });
            }
        }
    }
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
