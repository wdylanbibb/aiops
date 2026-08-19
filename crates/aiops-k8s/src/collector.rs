use aiops_core::{
    observations::{CollectionError, ObservationBundle, ObservationSource},
    resources::ResourceRef,
};
use chrono::{Duration, Utc};
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

        let mut bundle = ObservationBundle {
            target: target.clone(),
            collected_from: collected_at - self.options.lookback,
            collected_at,
            resources: vec![snapshot],
            logs: Vec::new(),
            events: Vec::new(),
            health: vec![pods::health(&pod)],
            errors: Vec::new(),
        };

        if self.options.include_events {
            match events::collect(self.client.inner(), &target).await {
                Ok(events) => bundle.events = events,
                Err(error) => bundle.errors.push(CollectionError {
                    resource: Some(target.clone()),
                    source: ObservationSource::Events,
                    message: error.to_string(),
                    retryable: is_retryable(&error),
                }),
            }
        }

        if self.options.include_logs {
            self.collect_container_logs(&pod, &target, &mut bundle)
                .await;
        }

        Ok(bundle)
    }

    async fn collect_container_logs(
        &self,
        pod: &Pod,
        target: &ResourceRef,
        bundle: &mut ObservationBundle,
    ) {
        let Some(namespace) = pod.metadata.namespace.as_deref() else {
            return;
        };

        let Some(name) = pod.metadata.name.as_deref() else {
            return;
        };

        for container in pod
            .spec
            .as_ref()
            .map(|spec| spec.containers.as_slice())
            .unwrap_or_default()
        {
            let request = PodLogRequest {
                namespace,
                pod: name,
                container: Some(&container.name),
                since_seconds: Some(self.options.lookback.num_seconds()),
                tail_lines: Some(self.options.tail_lines),
                previous: false,
            };

            match logs::collect(self.client.inner(), target.clone(), request).await {
                Ok(logs) => bundle.logs.extend(logs),
                Err(error) => bundle.errors.push(CollectionError {
                    resource: Some(target.clone()),
                    source: ObservationSource::Logs,
                    message: error.to_string(),
                    retryable: is_retryable(&error),
                }),
            }
        }
    }
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
