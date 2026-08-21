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

#[cfg(test)]
mod tests {
    use super::*;
    use aiops_core::resources::ResourceKind;

    fn resource() -> ResourceRef {
        ResourceRef {
            kind: ResourceKind::Pod,
            namespace: Some("default".into()),
            name: "api-0".into(),
            uid: Some("pod-uid".into()),
        }
    }

    #[test]
    fn parses_rfc3339_timestamp_and_preserves_log_metadata() {
        let entry = parse_line(
            resource(),
            Some("api"),
            true,
            "2026-01-02T03:04:05.123456789Z request completed",
        );

        assert_eq!(
            entry.timestamp.unwrap().to_rfc3339(),
            "2026-01-02T03:04:05.123456789+00:00"
        );
        assert_eq!(entry.message, "request completed");
        assert_eq!(entry.container.as_deref(), Some("api"));
        assert!(entry.previous_container);
        assert_eq!(entry.stream, LogStream::Unknown);
    }

    #[test]
    fn leaves_lines_without_valid_timestamp_untouched() {
        let entry = parse_line(resource(), None, false, "not-a-time original message");

        assert!(entry.timestamp.is_none());
        assert_eq!(entry.message, "not-a-time original message");
        assert!(entry.container.is_none());
        assert!(!entry.previous_container);
    }
}
