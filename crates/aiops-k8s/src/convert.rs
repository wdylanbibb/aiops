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
