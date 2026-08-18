use aiops_core::resources::ResourceKind;
use aiops_k8s::{client::KubernetesClient, convert::resource_ref, pods::list};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = KubernetesClient::infer().await?;

    for pod in list(client.inner(), Some("mediaserver-system"))
        .await?
        .iter()
        .map(|p| resource_ref(p, ResourceKind::Pod))
    {
        println!("{pod:#?}");
    }

    Ok(())
}
