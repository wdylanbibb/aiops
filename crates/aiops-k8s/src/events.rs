use aiops_core::{
    observations::ResourceEvent,
    resources::{ResourceKind, ResourceRef},
};
use k8s_openapi::api::core::v1::Event;
use kube::{Api, Client, api::ListParams};

use crate::convert::convert_event;

pub async fn collect(
    client: Client,
    target: &ResourceRef,
) -> Result<Vec<ResourceEvent>, kube::Error> {
    let events = match target.namespace.as_deref() {
        Some(namespace) => Api::<Event>::namespaced(client, namespace),
        None => Api::<Event>::all(client),
    };

    let selector = match &target.uid {
        Some(uid) => format!("involvedObject.uid={uid}"),
        None => format!(
            "involvedObject.kind={},involvedObject.name={}",
            kind_name(target),
            target.name
        ),
    };

    let listed = events
        .list(&ListParams::default().fields(&selector))
        .await?;

    Ok(listed
        .items
        .into_iter()
        .map(|event| convert_event(target, event))
        .collect())
}

fn kind_name(resource: &ResourceRef) -> String {
    match &resource.kind {
        ResourceKind::Custom(kind) => kind.clone(),
        kind => format!("{kind:?}"),
    }
}
