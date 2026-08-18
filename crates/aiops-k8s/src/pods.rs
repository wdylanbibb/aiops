use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client, api::ListParams};

pub async fn list(client: Client, namespace: Option<&str>) -> Result<Vec<Pod>, kube::Error> {
    let pods = match namespace {
        Some(ns) => Api::<Pod>::namespaced(client, ns),
        None => Api::<Pod>::all(client),
    };

    Ok(pods.list(&ListParams::default()).await?.items)
}
