//! Port-forwards: `kubectl port-forward` children that live in the
//! background for as long as knav does (or until stopped from `:pf`).

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::k8s::describe::{Col, Tone};
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
            labels: String::new(),
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

fn parse_port(text: &str, what: &str) -> Result<u16> {
    text.trim().parse::<u16>().ok().filter(|p| *p > 0).with_context(|| format!("{what}: '{}' is not a port (1-65535)", text.trim()))
}

/// Which part of the dialog has the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Container,
    Local,
    Address,
    Ok,
    Cancel,
}

/// The port-forward dialog: the container port, the local port, the address
/// to listen on, and OK / Cancel.
#[derive(Clone, Debug)]
pub struct PortForm {
    pub container: String,
    pub local: String,
    pub address: String,
    pub focus: Field,
    /// Ports the object declares, to warn when the one typed is not among them.
    pub declared: Vec<u16>,
    /// What was wrong with the last attempt to submit.
    pub error: Option<String>,
    /// Once the local port is typed by hand it stops following the container port.
    local_edited: bool,
}

impl PortForm {
    pub fn new(declared: Vec<u16>) -> Self {
        let first = declared.first().copied();
        PortForm {
            container: first.map(|p| p.to_string()).unwrap_or_default(),
            local: first.map(|p| suggested_local(p).to_string()).unwrap_or_default(),
            address: "localhost".into(),
            focus: Field::Container,
            declared,
            error: None,
            local_edited: false,
        }
    }

    const ORDER: [Field; 5] = [Field::Container, Field::Local, Field::Address, Field::Ok, Field::Cancel];

    fn step(&mut self, by: isize) {
        let at = Self::ORDER.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        self.focus = Self::ORDER[(at + by).rem_euclid(Self::ORDER.len() as isize) as usize];
    }

    pub fn next(&mut self) {
        self.step(1);
    }

    pub fn prev(&mut self) {
        self.step(-1);
    }

    fn field_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            Field::Container => Some(&mut self.container),
            Field::Local => Some(&mut self.local),
            Field::Address => Some(&mut self.address),
            Field::Ok | Field::Cancel => None,
        }
    }

    pub fn type_char(&mut self, c: char) {
        self.error = None;
        let allowed = match self.focus {
            Field::Container | Field::Local => c.is_ascii_digit() && self.field_mut().is_some_and(|f| f.len() < 5),
            Field::Address => (c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | ',')) && self.field_mut().is_some_and(|f| f.len() < 64),
            Field::Ok | Field::Cancel => false,
        };
        if allowed && let Some(field) = self.field_mut() {
            field.push(c);
        }
        self.after_edit();
    }

    pub fn backspace(&mut self) {
        self.error = None;
        if let Some(field) = self.field_mut() {
            field.pop();
        }
        self.after_edit();
    }

    fn after_edit(&mut self) {
        match self.focus {
            Field::Local => self.local_edited = true,
            Field::Container if !self.local_edited => {
                self.local = self.container.parse::<u16>().map(|p| suggested_local(p).to_string()).unwrap_or_default();
            }
            _ => {}
        }
    }

    /// What to show under the fields when the port typed is a guess.
    pub fn warning(&self) -> Option<String> {
        if self.declared.is_empty() {
            return Some("No ports declared here; make sure the app listens on it".into());
        }
        let typed = self.container.trim().parse::<u16>().ok()?;
        (!self.declared.contains(&typed)).then(|| format!("Not a declared port (declared: {})", self.declared.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")))
    }

    /// `(local, container, address)` ready for kubectl, or what to fix.
    pub fn parse(&self) -> Result<(u16, u16, String)> {
        let remote = parse_port(&self.container, "Container port")?;
        let local = if self.local.trim().is_empty() { suggested_local(remote) } else { parse_port(&self.local, "Local port")? };
        let address = if self.address.trim().is_empty() { "localhost" } else { self.address.trim() };
        Ok((local, remote, address.to_string()))
    }
}

/// The local port to suggest for a remote one: the same, or shifted above
/// 1024 when that would need root.
pub fn suggested_local(remote: u16) -> u16 {
    if remote >= 1024 { remote } else { remote + 8000 }
}

/// `start` off the UI thread, since it waits a moment to see whether kubectl fails.
pub async fn start_in_background(context: String, namespace: String, resource: String, address: String, local: u16, remote: u16) -> Result<Forward> {
    tokio::task::spawn_blocking(move || start(&context, &namespace, &resource, &address, local, remote)).await?
}

pub fn start(context: &str, namespace: &str, resource: &str, address: &str, local: u16, remote: u16) -> Result<Forward> {
    let mut child = Command::new("kubectl")
        .args(["--context", context, "-n", namespace, "port-forward", "--address", address, resource, &format!("{local}:{remote}")])
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
    // Nobody reads kubectl's later chatter, and an unread pipe would eventually stall it.
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut err, &mut std::io::sink());
        });
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

    fn form(declared: &[u16]) -> PortForm {
        PortForm::new(declared.to_vec())
    }

    #[test]
    fn the_form_starts_from_the_first_declared_port() {
        let f = form(&[80, 443]);
        assert_eq!((f.container.as_str(), f.local.as_str(), f.address.as_str()), ("80", "8080", "localhost"));
        assert_eq!(f.parse().unwrap(), (8080, 80, "localhost".to_string()));
        assert!(f.warning().is_none());
    }

    #[test]
    fn the_local_port_follows_the_container_port_until_it_is_typed() {
        let mut f = form(&[]);
        for c in "3000".chars() {
            f.type_char(c);
        }
        assert_eq!(f.local, "3000");
        f.focus = Field::Local;
        f.backspace();
        f.type_char('1');
        for _ in 0..2 {
            f.prev();
        }
        f.type_char('5');
        assert_eq!(f.local, "3001", "hand-edited, so it no longer follows");
    }

    #[test]
    fn a_guessed_port_gets_a_warning_and_a_declared_one_does_not() {
        let mut f = form(&[]);
        assert!(f.warning().unwrap().contains("No ports declared"), "even before anything is typed");
        f.type_char('9');
        assert!(f.warning().unwrap().contains("No ports declared"));
        let mut g = form(&[80]);
        g.backspace();
        g.backspace();
        g.type_char('9');
        assert!(g.warning().unwrap().contains("declared: 80"));
    }

    #[test]
    fn bad_input_is_named_and_blank_fields_fall_back_to_defaults() {
        let mut f = form(&[]);
        assert!(f.parse().unwrap_err().to_string().contains("Container port"));
        f.type_char('8');
        f.type_char('0');
        f.address.clear();
        assert_eq!(f.parse().unwrap(), (8080, 80, "localhost".to_string()));
        f.container = "70000".into();
        assert!(f.parse().is_err());
    }

    #[test]
    fn tab_cycles_through_the_fields_and_buttons() {
        let mut f = form(&[]);
        let mut seen = vec![f.focus];
        for _ in 0..5 {
            f.next();
            seen.push(f.focus);
        }
        assert_eq!(seen, [Field::Container, Field::Local, Field::Address, Field::Ok, Field::Cancel, Field::Container]);
        f.prev();
        assert_eq!(f.focus, Field::Cancel);
    }

    #[test]
    fn only_digits_go_into_port_fields() {
        let mut f = form(&[]);
        f.type_char('a');
        f.type_char('8');
        assert_eq!(f.container, "8");
    }

    #[test]
    fn privileged_remote_ports_get_a_high_local_suggestion() {
        assert_eq!(suggested_local(80), 8080);
        assert_eq!(suggested_local(8443), 8443);
    }
}
