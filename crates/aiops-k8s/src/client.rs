use kube::Client;

#[derive(Clone)]
pub struct KubernetesClient {
    inner: Client,
}

impl KubernetesClient {
    pub async fn infer() -> Result<Self, kube::Error> {
        let inner = Client::try_default().await?;
        Ok(Self { inner })
    }

    pub fn inner(&self) -> Client {
        self.inner.clone()
    }
}

impl From<Client> for KubernetesClient {
    fn from(value: Client) -> Self {
        Self { inner: value }
    }
}
