//! Port-forwards: `kubectl port-forward` children that live in the
//! background for as long as knav does (or until stopped from `:pf`).

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

pub struct Forward {
    /// `pod/web-1`, `svc/api`, `deploy/web`.
    pub resource: String,
    pub namespace: String,
    pub local: u16,
    pub remote: u16,
    child: Child,
}

impl Forward {
    pub fn label(&self) -> String {
        format!("localhost:{} → {}/{}:{}", self.local, self.namespace, self.resource, self.remote)
    }

    /// False once kubectl has exited (the pod went away, the connection dropped).
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `8080:80` (local:remote) or a bare `80` for the same port on both sides.
pub fn parse_ports(text: &str) -> Result<(u16, u16)> {
    let port = |s: &str| s.trim().parse::<u16>().ok().filter(|p| *p > 0).with_context(|| format!("'{}' is not a port", s.trim()));
    match text.split_once(':') {
        Some((local, remote)) => Ok((port(local)?, port(remote)?)),
        None => port(text).map(|p| (p, p)),
    }
}

/// The local port to suggest for a remote one: the same, or shifted above
/// 1024 when that would need root.
pub fn suggested_local(remote: u16) -> u16 {
    if remote >= 1024 { remote } else { remote + 8000 }
}

pub fn start(context: &str, namespace: &str, resource: &str, local: u16, remote: u16) -> Result<Forward> {
    let mut child = Command::new("kubectl")
        .args(["--context", context, "-n", namespace, "port-forward", resource, &format!("{local}:{remote}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("couldn't run kubectl (is it on the PATH?)")?;
    // Give it a moment to fail (port taken, no such pod) before reporting success.
    std::thread::sleep(Duration::from_millis(800));
    if let Ok(Some(status)) = child.try_wait() {
        let mut message = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut err, &mut message);
        }
        bail!("{}", if message.trim().is_empty() { format!("kubectl exited with {status}") } else { message.trim().to_string() });
    }
    Ok(Forward { resource: resource.to_string(), namespace: namespace.to_string(), local, remote, child })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_parse_as_local_colon_remote_or_one_number() {
        assert_eq!(parse_ports("8080:80").unwrap(), (8080, 80));
        assert_eq!(parse_ports(" 3000 ").unwrap(), (3000, 3000));
    }

    #[test]
    fn bad_ports_are_rejected() {
        for bad in ["", "abc", "80:", ":80", "0", "70000", "1:2:3"] {
            assert!(parse_ports(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn privileged_remote_ports_get_a_high_local_suggestion() {
        assert_eq!(suggested_local(80), 8080);
        assert_eq!(suggested_local(8443), 8443);
    }
}
