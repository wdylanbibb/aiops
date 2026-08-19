use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use kube::{Api, Client, api::ListParams};

pub struct Workloads {
    pub deployments: Vec<Deployment>,
    pub stateful_sets: Vec<StatefulSet>,
    pub daemon_sets: Vec<DaemonSet>,
}

pub async fn list(client: Client, namespace: Option<&str>) -> Result<Workloads, kube::Error> {
    let deployments = match namespace {
        Some(namespace) => Api::<Deployment>::namespaced(client.clone(), namespace),
        None => Api::<Deployment>::all(client.clone()),
    };

    let stateful_sets = match namespace {
        Some(namespace) => Api::<StatefulSet>::namespaced(client.clone(), namespace),
        None => Api::<StatefulSet>::all(client.clone()),
    };

    let daemon_sets = match namespace {
        Some(namespace) => Api::<DaemonSet>::namespaced(client.clone(), namespace),
        None => Api::<DaemonSet>::all(client.clone()),
    };

    let params = ListParams::default();

    let (deployments, stateful_sets, daemon_sets) = tokio::try_join!(
        deployments.list(&params),
        stateful_sets.list(&params),
        daemon_sets.list(&params),
    )?;

    Ok(Workloads {
        deployments: deployments.items,
        stateful_sets: stateful_sets.items,
        daemon_sets: daemon_sets.items,
    })
}
