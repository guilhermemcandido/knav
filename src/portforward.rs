//! Port-forwards: `kubectl port-forward` children that live in the
//! background for as long as knav does (or until stopped from `:pf`).

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::describe::{Col, Tone};
use crate::k8s::GenericRow;

use anyhow::{Context as _, Result, bail};

pub struct Forward {
    /// `pod/web-1`, `svc/api`, `deploy/web`.
    pub resource: String,
    pub namespace: String,
    pub local: u16,
    pub remote: u16,
    child: Child,
    started: Instant,
}

/// The extra columns of the Port-forwards list, between NAME and AGE.
pub const HEADERS: [&str; 3] = ["LOCAL", "REMOTE", "URL"];

impl Forward {
    pub fn label(&self) -> String {
        format!("localhost:{} → {}/{}:{}", self.local, self.namespace, self.resource, self.remote)
    }

    pub fn url(&self) -> String {
        format!("http://localhost:{}", self.local)
    }

    /// This forward as a row of the Port-forwards list.
    pub fn row(&self) -> GenericRow {
        let secs = self.started.elapsed().as_secs() as i64;
        let col = |header, text: String, sort| Col { header, text, tone: Tone::Plain, sort };
        GenericRow {
            namespace: self.namespace.clone(),
            name: self.resource.clone(),
            age: crate::k8s::humanize_age(k8s_openapi::jiff::Timestamp::now() - Duration::from_secs(secs as u64)),
            age_secs: secs,
            extras: vec![
                col("LOCAL", self.local.to_string(), Some(i64::from(self.local))),
                col("REMOTE", self.remote.to_string(), Some(i64::from(self.remote))),
                col("URL", self.url(), None),
            ],
            status: Some((Tone::Good, self.label())),
            uid: format!("forward-{}", self.local),
            owners: Vec::new(),
        }
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
    Ok(Forward { resource: resource.to_string(), namespace: namespace.to_string(), local, remote, child, started: Instant::now() })
}

/// Opens `url` in the default browser.
pub fn open_in_browser(url: &str) -> Result<()> {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/c", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .with_context(|| format!("couldn't open {url} with {program}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_is_a_row_of_the_port_forwards_list() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut forward = Forward { resource: "pod/web".into(), namespace: "shop".into(), local: 8080, remote: 80, child, started: Instant::now() };
        let row = forward.row();
        assert_eq!((row.namespace.as_str(), row.name.as_str()), ("shop", "pod/web"));
        let cells: Vec<(&str, &str)> = row.extras.iter().map(|c| (c.header, c.text.as_str())).collect();
        assert_eq!(cells, [("LOCAL", "8080"), ("REMOTE", "80"), ("URL", "http://localhost:8080")]);
        assert_eq!(row.extras.len(), HEADERS.len());
        assert_eq!(row.extras[0].sort, Some(8080));
        assert!(forward.alive());
        assert_eq!(forward.label(), "localhost:8080 → shop/pod/web:80");
    }

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
