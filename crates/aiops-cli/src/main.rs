use aiops_k8s::{
    client::KubernetesClient,
    pods::{health, list},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = KubernetesClient::infer().await?;

    for pod in list(client.inner(), Some("mediaserver-system")).await? {
        let Some(namespace) = pod.metadata.namespace.as_deref() else {
            continue;
        };

        let Some(name) = pod.metadata.name.as_deref() else {
            continue;
        };

        for container in pod
            .spec
            .as_ref()
            .map(|spec| spec.containers.as_slice())
            .unwrap_or_default()
        {
            let request = aiops_k8s::logs::PodLogRequest {
                namespace,
                pod: name,
                container: Some(&container.name),
                since_seconds: Some(900),
                tail_lines: Some(500),
                previous: false,
            };

            // for log in aiops_k8s::logs::collect(
            //     client.inner(),
            //     aiops_k8s::convert::resource_ref(&pod, aiops_core::resources::ResourceKind::Pod),
            //     request,
            // )
            // .await?
            // {
            //     println!("{log:#?}");
            // }
            
            for event in aiops_k8s::events::collect(client.inner(), &aiops_k8s::convert::resource_ref(&pod, aiops_core::resources::ResourceKind::Pod)).await? {
                println!("{event:#?}");
            }
        }
    }

    Ok(())
}
