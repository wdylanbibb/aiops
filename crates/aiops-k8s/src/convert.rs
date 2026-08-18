use aiops_core::resources::{ResourceKind, ResourceRef};
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
