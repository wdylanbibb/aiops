use aiops_k8s::{
    client::KubernetesClient,
    pods::{list, health},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = KubernetesClient::infer().await?;

    for pod in list(client.inner(), Some("mediaserver-system"))
        .await?
        .iter()
        .map(health)
    {
        println!("{pod:#?}");
    }

    Ok(())
}
