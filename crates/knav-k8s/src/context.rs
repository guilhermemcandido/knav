
use anyhow::Result;
use kube::Client;

/// One kubeconfig context for the picker. Cluster and namespace are shown too,
/// since similar names can point at very different clusters.
pub struct ContextInfo {
    pub name: String,
    pub cluster: String,
    pub user: String,
    /// The namespace the context defaults to, when it sets one.
    pub namespace: Option<String>,
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
            let namespace = c.context.as_ref().and_then(|ctx| ctx.namespace.clone());
            let is_current = current.as_deref() == Some(c.name.as_str());
            ContextInfo { name: c.name, cluster, user, namespace, is_current }
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

/// Fails fast if the API server can't be reached; otherwise the reflectors retry
/// silently and nothing is drawn. Returns the server version.
pub async fn ensure_reachable(client: &Client) -> Result<String> {
    match tokio::time::timeout(std::time::Duration::from_secs(5), client.apiserver_version()).await {
        Ok(Ok(info)) => Ok(info.git_version),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => anyhow::bail!("timed out"),
    }
}

/// Connects to `context` and checks the cluster answers, saying why in plain words
/// when it doesn't. Returns the client and the server's version.
pub async fn connect_checked(context: Option<&str>) -> std::result::Result<(Client, String), Unreachable> {
    let label = context.map(str::to_string).or_else(|| list_contexts().ok()?.into_iter().find(|c| c.is_current).map(|c| c.name)).unwrap_or_default();
    let attempt = async {
        let client = connect_to_context(context).await?;
        let version = ensure_reachable(&client).await?;
        anyhow::Ok((client, version))
    };
    attempt.await.map_err(|e| Unreachable::explain(&label, &format!("{e:#}"), aws_profile(&label).as_deref()))
}

/// Why a context couldn't be opened, in a line, and what to do about it.
#[derive(Debug, Clone)]
pub struct Unreachable {
    pub context: String,
    pub reason: String,
    pub fix: String,
}

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n{}", self.reason, self.fix)
    }
}

impl std::error::Error for Unreachable {}

impl Unreachable {
    /// Reads `error` (the whole chain, with a credential plugin's output) for the usual
    /// causes: an expired cloud login, refused credentials, an unreachable server.
    pub fn explain(context: &str, error: &str, aws_profile: Option<&str>) -> Self {
        let text = error.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
        let (reason, fix) = if has(&["sso"]) && has(&["expired", "refresh failed", "sso login", "invalid_grant"]) {
            let profile = aws_profile.map(|p| format!(" --profile {p}")).unwrap_or_default();
            ("Your AWS SSO login has expired".to_string(), format!("Run `aws sso login{profile}` and try again"))
        } else if has(&["expiredtoken", "token included in the request is expired", "requestexpired"]) {
            ("Your AWS credentials have expired".to_string(), "Refresh them (aws sso login, or new keys) and try again".to_string())
        } else if has(&["gke-gcloud-auth-plugin", "gcloud auth"]) {
            ("Your Google Cloud login has expired".to_string(), "Run `gcloud auth login` and try again".to_string())
        } else if has(&["kubelogin", "az login", "aadsts"]) {
            ("Your Azure login has expired".to_string(), "Run `az login` and try again".to_string())
        } else if has(&["exec", "auth plugin", "credential"]) && !has(&["401", "unauthorized"]) {
            ("Couldn't get credentials for this cluster".to_string(), first_useful_line(error))
        } else if has(&["401", "unauthorized"]) {
            ("The cluster refused your credentials".to_string(), "They may have expired: log in again and try again".to_string())
        } else if has(&["403", "forbidden"]) {
            ("Your user can't access this cluster".to_string(), first_useful_line(error))
        } else if has(&["certificate", "tls", "x509"]) {
            ("The cluster's certificate isn't trusted".to_string(), first_useful_line(error))
        } else if has(&["timed out", "timeout"]) {
            ("The cluster didn't answer in time".to_string(), "Is it running, or does it need a VPN?".to_string())
        } else if has(&["connection refused", "dns", "lookup", "no route", "unreachable", "connect"]) {
            ("Can't reach the cluster's API server".to_string(), "Is it running, or does it need a VPN?".to_string())
        } else {
            ("Can't connect to this cluster".to_string(), first_useful_line(error))
        };
        Unreachable { context: context.to_string(), reason, fix }
    }
}

/// The first line of an error worth showing: not blank, not a stack frame, kept short.
fn first_useful_line(error: &str) -> String {
    let line = error.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("at ")).unwrap_or("unknown error");
    let short: String = line.chars().take(110).collect();
    if short.len() < line.len() { format!("{short}…") } else { short }
}

/// The AWS profile a context's credential plugin uses, for the `aws sso login` hint.
fn aws_profile(context: &str) -> Option<String> {
    let config = kube::config::Kubeconfig::read().ok()?;
    let user = config.contexts.iter().find(|c| c.name == context)?.context.as_ref()?.user.clone()?;
    let exec = config.auth_infos.iter().find(|a| a.name == user)?.auth_info.as_ref()?.exec.as_ref()?;
    let from_env = exec.env.iter().flatten().find(|e| e.get("name").map(String::as_str) == Some("AWS_PROFILE")).and_then(|e| e.get("value").cloned());
    let args = exec.args.clone().unwrap_or_default();
    let from_args = args.iter().position(|a| a == "--profile").and_then(|i| args.get(i + 1).cloned());
    from_env.or(from_args)
}

#[cfg(test)]
mod explain_tests {
    use super::*;

    #[test]
    fn an_expired_aws_sso_login_says_how_to_log_in() {
        let error = "failed to get token: exec plugin: command aws failed: Error when retrieving token from sso: Token has expired and refresh failed";
        let u = Unreachable::explain("prod", error, Some("platform"));
        assert_eq!(u.reason, "Your AWS SSO login has expired");
        assert!(u.fix.contains("aws sso login --profile platform"), "{}", u.fix);
    }

    #[test]
    fn refused_credentials_and_a_dead_server_read_plainly() {
        assert_eq!(Unreachable::explain("c", "ApiError: Unauthorized (401)", None).reason, "The cluster refused your credentials");
        assert_eq!(Unreachable::explain("c", "error trying to connect: tcp connect error: Connection refused", None).reason, "Can't reach the cluster's API server");
        assert_eq!(Unreachable::explain("c", "timed out", None).reason, "The cluster didn't answer in time");
    }

    #[test]
    fn anything_else_keeps_a_short_first_line() {
        let u = Unreachable::explain("c", "\nsomething odd happened\n  at frame", None);
        assert_eq!(u.fix, "something odd happened");
    }
}
