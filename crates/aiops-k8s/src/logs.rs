use aiops_core::{
    observations::{LogEntry, LogStream},
    resources::ResourceRef,
};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client, api::LogParams};

pub struct PodLogRequest<'a> {
    pub namespace: &'a str,
    pub pod: &'a str,
    pub container: Option<&'a str>,
    pub since_seconds: Option<i64>,
    pub tail_lines: Option<i64>,
    pub previous: bool,
}

pub async fn collect(
    client: Client,
    resource: ResourceRef,
    request: PodLogRequest<'_>,
) -> Result<Vec<LogEntry>, kube::Error> {
    let pods = Api::<Pod>::namespaced(client, request.namespace);

    let params = LogParams {
        container: request.container.map(str::to_owned),
        since_seconds: request.since_seconds,
        tail_lines: request.tail_lines,
        previous: request.previous,
        timestamps: true,
        ..Default::default()
    };

    let output = pods.logs(request.pod, &params).await?;

    Ok(output
        .lines()
        .map(|line| parse_line(resource.clone(), request.container, request.previous, line))
        .collect())
}

fn parse_line(
    resource: ResourceRef,
    container: Option<&str>,
    previous: bool,
    line: &str,
) -> LogEntry {
    let (timestamp, message) = line
        .split_once(' ')
        .and_then(|(candidate, message)| {
            candidate
                .parse::<DateTime<Utc>>()
                .ok()
                .map(|timestamp| (Some(timestamp), message))
        })
        .unwrap_or((None, line));

    LogEntry {
        resource,
        timestamp,
        container: container.map(str::to_owned),
        stream: LogStream::Unknown,
        message: message.to_owned(),
        previous_container: previous,
    }
}
