
use anyhow::Result;
use kube::Client;

/// One kubeconfig context, for the cluster picker — just enough to list
/// and identify it. `cluster`/`namespace` are shown alongside the name
/// since two contexts can share a name pattern (e.g. "prod-us"/"prod-eu")
/// but point at very different clusters, which the name alone wouldn't
/// make obvious.
pub struct ContextInfo {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub is_current: bool,
}

/// Every context in the kubeconfig (`$KUBECONFIG` or `~/.kube/config`,
/// same resolution `kube` itself uses) — the picker's whole candidate
/// list. Ordering matches the file, same as `kubectl config get-contexts`.
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

/// Connects to a specific kubeconfig context by name, or (`None`) whatever
/// `kube` itself would infer — in-cluster config if running inside a pod,
/// else the kubeconfig's own `current-context`. The same "infer" path
/// `connect` already used, just exposed so a chosen context can override it.
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

/// Fails fast, with a readable message, if the API server can't be
/// reached — without this, an unreachable cluster just hangs forever in
/// the reflectors' initial list (which retry silently), and knav never
/// draws anything. `context` is only for the message. Returns the
/// server's version (`v1.35.5+k3s1`).
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
