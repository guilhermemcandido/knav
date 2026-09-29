
use anyhow::Result;
use kube::Client;

/// One kubeconfig context for the picker. Cluster and namespace are shown too,
/// since similar names can point at very different clusters.
pub struct ContextInfo {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub is_current: bool,
}

/// Every context in the kubeconfig, resolved the way `kube` does, in file order.
pub fn list_contexts() -> Result<Vec<ContextInfo>> {
    let kubeconfig = kube::config::Kubeconfig::read()?;
    let current = kubeconfig.current_context.clone();
    Ok(kubeconfig
        .contexts
        .into_iter()
        .map(|c| {
            let cluster = c.context.as_ref().map(|ctx| ctx.cluster.clone()).unwrap_or_default();
            let user = c.context.as_ref().and_then(|ctx| ctx.user.clone()).unwrap_or_default();
            let is_current = current.as_deref() == Some(c.name.as_str());
            ContextInfo { name: c.name, cluster, user, is_current }
        })
        .collect())
}

/// Connects to a kubeconfig context by name, or (`None`) whatever `kube` infers:
/// in-cluster config, else the kubeconfig's `current-context`.
pub async fn connect_to_context(context: Option<&str>) -> Result<Client> {
    let config = match context {
        Some(name) => {
            kube::Config::from_kubeconfig(&kube::config::KubeConfigOptions { context: Some(name.to_string()), ..Default::default() })
                .await?
        }
        None => kube::Config::infer().await?,
    };
    Ok(Client::try_from(config)?)
}

/// Fails fast with a readable message if the API server is unreachable; otherwise
/// the reflectors retry silently and nothing is drawn. Returns the server version.
pub async fn ensure_reachable(client: &Client, context: Option<&str>) -> Result<String> {
    let label = match context {
        Some(name) => name.to_string(),
        None => list_contexts()
            .ok()
            .and_then(|c| c.into_iter().find(|c| c.is_current).map(|c| c.name))
            .unwrap_or_else(|| "the current context".to_string()),
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), client.apiserver_version()).await {
        Ok(Ok(info)) => Ok(info.git_version),
        Ok(Err(e)) => anyhow::bail!(
            "can't reach cluster '{label}': {e}\nIs it running? Try `knav -c <context>`."
        ),
        Err(_) => anyhow::bail!(
            "can't reach cluster '{label}': timed out\nIs it running? Try `knav -c <context>`."
        ),
    }
}
