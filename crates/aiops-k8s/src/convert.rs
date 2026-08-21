use aiops_core::{
    observations::{EventType, ResourceEvent},
    resources::{ResourceKind, ResourceRef},
};
use chrono::DateTime;
use k8s_openapi::api::core::v1::Event;
use kube::{Resource, ResourceExt};

pub fn resource_ref<K>(resource: &K, kind: ResourceKind) -> ResourceRef
where
    K: Resource + ResourceExt,
{
    ResourceRef {
        kind,
        namespace: resource.namespace(),
        name: resource.name_any(),
        uid: resource.uid(),
    }
}

pub fn resource_kind(kind: &str) -> ResourceKind {
    match kind {
        "Pod" => ResourceKind::Pod,
        "Deployment" => ResourceKind::Deployment,
        "StatefulSet" => ResourceKind::StatefulSet,
        "DaemonSet" => ResourceKind::DaemonSet,
        "ReplicaSet" => ResourceKind::ReplicaSet,
        "Job" => ResourceKind::Job,
        "CronJob" => ResourceKind::CronJob,
        "Service" => ResourceKind::Service,
        "Ingress" => ResourceKind::Ingress,
        "ConfigMap" => ResourceKind::ConfigMap,
        "Secret" => ResourceKind::Secret,
        "PersistentVolume" => ResourceKind::PersistentVolume,
        "PersistentVolumeClaim" => ResourceKind::PersistentVolumeClaim,
        "Node" => ResourceKind::Node,
        "Namespace" => ResourceKind::Namespace,
        other => ResourceKind::Custom(other.to_owned()),
    }
}

pub(crate) fn convert_event(target: &ResourceRef, event: Event) -> ResourceEvent {
    ResourceEvent {
        regarding: target.clone(),
        reporting_controller: event.reporting_component,
        event_type: match event.type_.as_deref() {
            Some("Normal") => EventType::Normal,
            Some("Warning") => EventType::Warning,
            _ => EventType::Unknown,
        },
        reason: event.reason,
        message: event.message.unwrap_or_default(),
        first_seen: event.first_timestamp.map(|time| {
            let secs = time.0.as_second();
            let nsecs = time.0.subsec_nanosecond() as u32;

            DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
        }),
        last_seen: event.last_timestamp.map(|time| {
            let secs = time.0.as_second();
            let nsecs = time.0.subsec_nanosecond() as u32;

            DateTime::from_timestamp(secs, nsecs).unwrap_or_default()
        }),
        count: event.count.unwrap_or(1).max(0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    #[test]
    fn maps_builtin_and_custom_resource_kinds() {
        assert_eq!(resource_kind("Deployment"), ResourceKind::Deployment);
        assert_eq!(resource_kind("Pod"), ResourceKind::Pod);
        assert_eq!(
            resource_kind("Widget"),
            ResourceKind::Custom("Widget".into())
        );
    }

    #[test]
    fn resource_ref_uses_kubernetes_metadata() {
        let pod = k8s_openapi::api::core::v1::Pod {
            metadata: ObjectMeta {
                name: Some("api-0".into()),
                namespace: Some("production".into()),
                uid: Some("abc123".into()),
                ..Default::default()
            },
            ..Default::default()
        };

        let reference = resource_ref(&pod, ResourceKind::Pod);
        assert_eq!(reference.name, "api-0");
        assert_eq!(reference.namespace.as_deref(), Some("production"));
        assert_eq!(reference.uid.as_deref(), Some("abc123"));
    }

    #[test]
    fn converts_unknown_events_and_sanitizes_negative_counts() {
        let target = ResourceRef {
            kind: ResourceKind::Pod,
            namespace: Some("default".into()),
            name: "api-0".into(),
            uid: None,
        };
        let converted = convert_event(
            &target,
            Event {
                type_: Some("Other".into()),
                message: None,
                count: Some(-3),
                ..Default::default()
            },
        );

        assert_eq!(converted.regarding, target);
        assert_eq!(converted.event_type, EventType::Unknown);
        assert_eq!(converted.message, "");
        assert_eq!(converted.count, 0);
    }
}
