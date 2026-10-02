//! `knav update`: replaces this binary with the latest GitHub release when newer, or
//! with a rebuild of the same version when its checksum differs from this binary's,
//! listing the commits in between.
//! Uses `curl` rather than an HTTP client crate for this one command.

use anyhow::{Context as _, Result};

use std::io::Write;

use sha2::{Digest, Sha256};

const REPO: &str = "guilhermemcandido/knav";

/// The commit this binary was built from, set by build.rs (empty when unknown).
const COMMIT: &str = env!("KNAV_COMMIT");

/// How many commit subjects "What's new" lists before saying how many more.
const MAX_CHANGES: usize = 12;

/// Colours for a terminal, nothing when piped or with NO_COLOR set.
struct Paint(bool);

impl Paint {
    fn detect() -> Self {
        use std::io::IsTerminal;
        Paint(std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none())
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.0 { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }

    fn ok(&self, text: &str) -> String {
        self.wrap("32", text)
    }

    fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }

    fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }

    fn accent(&self, text: &str) -> String {
        self.wrap("36", text)
    }

    /// Replaces the line being drawn, for a step that finished.
    fn done(&self, text: &str) {
        let clear = if self.0 { "\r\x1b[2K" } else { "" };
        println!("{clear}  {} {text}", self.ok("✔"));
    }

    /// A step in progress, replaced by `done` when it finishes.
    fn working(&self, text: &str) {
        if self.0 {
            print!("  {} {text}", self.dim("…"));
            let _ = std::io::stdout().flush();
        }
    }
}

/// `0.1.0 · af347e2`, or just the version when the commit is unknown.
fn build_name(version: &str, commit: &str) -> String {
    match short(commit) {
        "" => version.to_string(),
        sha => format!("{version} · {sha}"),
    }
}

fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// Runs the update, and on failure says why on a red line, like the steps that worked.
pub(crate) fn run(auto_yes: bool) -> Result<()> {
    let paint = Paint::detect();
    if let Err(error) = update(&paint, auto_yes) {
        let clear = if paint.0 { "\r\x1b[2K" } else { "" };
        eprintln!("{clear}  {} {error:#}", paint.wrap("31", "✘"));
        std::process::exit(1);
    }
    Ok(())
}

fn update(paint: &Paint, auto_yes: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    paint.working("Checking for updates");
    let release = latest_release()?;
    let latest = release.tag.clone();
    let target = target_triple().with_context(|| format!("no prebuilt binary for {}/{}; reinstall with `cargo install --git https://github.com/{REPO}`", std::env::consts::OS, std::env::consts::ARCH))?;
    let exe = running_exe()?;
    let checksum = download(&format!("https://github.com/{REPO}/releases/download/{latest}/knav-{target}.sha256"))?;
    // Releases before this was added have no commit file.
    let latest_commit = download(&format!("https://github.com/{REPO}/releases/download/{latest}/knav-commit")).map(|b| String::from_utf8_lossy(&b).trim().to_string()).unwrap_or_default();
    let new_version = version(&latest) != current;
    if paint.0 {
        print!("\r\x1b[2K");
    }
    // The same commit counts too: a local build of it never matches byte for byte.
    let same_commit = !COMMIT.is_empty() && COMMIT == latest_commit;
    if !new_version && (same_commit || same_build(&exe, &checksum)) {
        paint.done(&format!("knav {} is up to date", paint.bold(&build_name(current, COMMIT))));
        return Ok(());
    }
    let compared = compare(COMMIT, &latest_commit);
    // A local build past the release: nothing to update to.
    if !new_version && compared.as_ref().is_some_and(|c| c.ahead) {
        paint.done(&format!("knav {} is newer than the latest build ({})", paint.bold(&build_name(current, COMMIT)), short(&latest_commit)));
        return Ok(());
    }

    println!();
    println!("  {}  {}", paint.dim("Current"), build_name(current, COMMIT));
    // Without a comparison (a commit GitHub doesn't have yet) the order is unknown.
    let note = match (new_version, &compared) {
        (true, _) => "new version",
        (false, Some(_)) => "newer build",
        (false, None) => "different build",
    };
    println!("  {}   {}  {}", paint.dim("Latest"), paint.bold(&build_name(version(&latest), &latest_commit)), paint.accent(note));
    let changes = compared.map(|c| c.subjects).unwrap_or_else(|| release_notes(&release.body));
    if !changes.is_empty() {
        println!();
        println!("  {}", paint.bold("What's new"));
        for line in changes.iter().take(MAX_CHANGES) {
            println!("    {} {line}", paint.accent("•"));
        }
        if changes.len() > MAX_CHANGES {
            println!("    {}", paint.dim(&format!("and {} more", changes.len() - MAX_CHANGES)));
        }
    }
    println!();
    if !auto_yes && !confirm(&format!("  Update now? {} ", paint.dim("[y/N]")))? {
        return Ok(());
    }
    refuse_if_package_managed(&exe)?;

    paint.working(&format!("Downloading knav-{target}"));
    let bytes = download(&format!("https://github.com/{REPO}/releases/download/{latest}/knav-{target}"))?;
    paint.done(&format!("Downloaded {}", paint.dim(&megabytes(bytes.len()))));
    verify(&bytes, &checksum)?;
    paint.done("Checksum verified");
    replace_running_binary(&exe, &bytes)?;
    paint.done(&format!("{} {} {} → {}", paint.ok(&paint.bold("Update installed")), paint.dim("·"), build_name(current, COMMIT), paint.bold(&build_name(version(&latest), &latest_commit))));
    println!("    {}", paint.dim("Run knav again to use it."));
    Ok(())
}

fn megabytes(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// How the running build relates to the release's: the commit subjects in between,
/// newest first, and whether the running one is ahead.
struct Compared {
    subjects: Vec<String>,
    ahead: bool,
}

/// `None` when either commit is unknown or GitHub can't say.
fn compare(from: &str, to: &str) -> Option<Compared> {
    if from.is_empty() || to.is_empty() || from == to {
        return None;
    }
    let body = download(&format!("https://api.github.com/repos/{REPO}/compare/{from}...{to}")).ok()?;
    compared_from_json(&body)
}

fn compared_from_json(body: &[u8]) -> Option<Compared> {
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let ahead = json.get("status").and_then(|s| s.as_str()) == Some("behind");
    let commits = json.get("commits")?.as_array()?;
    let mut subjects: Vec<String> = commits.iter().filter_map(|c| c.get("commit")?.get("message")?.as_str()).filter_map(|m| m.lines().next()).map(str::to_string).collect();
    subjects.reverse();
    Some(Compared { subjects, ahead })
}

/// The bullet lines of a release's notes, for when commits can't be compared.
fn release_notes(body: &str) -> Vec<String> {
    body.lines().filter_map(|l| l.trim().strip_prefix("* ").or_else(|| l.trim().strip_prefix("- "))).map(|l| l.split(" by @").next().unwrap_or(l).to_string()).collect()
}

struct Release {
    tag: String,
    body: String,
}

/// The tag a release is fetched at, without its leading `v` (`v0.2.0` -> `0.2.0`).
fn version(tag: &str) -> &str {
    tag.trim_start_matches('v')
}

/// The `<arch>-<os>` part of the release asset name for this platform.
fn target_triple() -> Option<&'static str> {
    triple_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// The name `.github/workflows/release.yml` gives each target it builds.
fn triple_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        _ => None,
    }
}

/// The release GitHub reports as the latest, e.g. `v0.2.0`, with its notes.
fn latest_release() -> Result<Release> {
    let body = download(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    let tag = tag_from_release_json(&body)?;
    let notes = serde_json::from_slice::<serde_json::Value>(&body).ok().and_then(|j| j.get("body")?.as_str().map(String::from)).unwrap_or_default();
    Ok(Release { tag, body: notes })
}

fn tag_from_release_json(body: &[u8]) -> Result<String> {
    let json: serde_json::Value = serde_json::from_slice(body).context("couldn't read GitHub's response")?;
    json.get("tag_name").and_then(|t| t.as_str()).map(str::to_string).context("GitHub's response had no tag_name (rate limited, or there is no release yet)")
}

fn download(url: &str) -> Result<Vec<u8>> {
    let output = std::process::Command::new("curl").args(["-fsSL", url]).output().context("couldn't run curl (is it installed?)")?;
    if !output.status.success() {
        anyhow::bail!("couldn't download {url}");
    }
    Ok(output.stdout)
}

/// Checks `bytes` against a `<hex>  <name>` checksum file's first field.
fn verify(bytes: &[u8], checksum_file: &[u8]) -> Result<()> {
    let text = String::from_utf8_lossy(checksum_file);
    let expected = text.split_whitespace().next().context("empty checksum file")?;
    let actual = hex_sha256(bytes);
    if actual != expected.to_lowercase() {
        anyhow::bail!("checksum mismatch (expected {expected}, got {actual}); not installing");
    }
    Ok(())
}

/// Whether the running binary is the one the release's checksum file describes.
fn same_build(exe: &std::path::Path, checksum_file: &[u8]) -> bool {
    std::fs::read(exe).is_ok_and(|bytes| verify(&bytes, checksum_file).is_ok())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn running_exe() -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe().context("couldn't find the running binary's path")?;
    Ok(exe.canonicalize().unwrap_or(exe))
}

/// Refuses a binary under a package manager's directory: that manager should update it.
fn refuse_if_package_managed(exe: &std::path::Path) -> Result<()> {
    let path = exe.to_string_lossy();
    if path.contains("/Cellar/") || path.contains("/homebrew/") {
        anyhow::bail!("knav was installed with Homebrew; run `brew upgrade knav` instead");
    }
    if path.contains("/nix/store/") {
        anyhow::bail!("knav was installed with Nix; update it through your Nix config instead");
    }
    Ok(())
}

/// Writes `bytes` beside `exe` and renames over it. On Unix a rename only repoints the
/// directory entry, so this is safe while `exe` is running.
fn replace_running_binary(exe: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let dir = exe.parent().context("the running binary has no parent directory")?;
    let tmp = dir.join(".knav-update");
    {
        let mut file = std::fs::File::create(&tmp).with_context(|| format!("couldn't write to {} (try `sudo knav update`?)", dir.display()))?;
        file.write_all(bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
    }
    std::fs::rename(&tmp, exe).with_context(|| format!("couldn't replace {}", exe.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_is_read_from_the_releases_api_shape() {
        let body = br#"{"tag_name": "v0.2.0", "name": "v0.2.0"}"#;
        assert_eq!(tag_from_release_json(body).unwrap(), "v0.2.0");
    }

    #[test]
    fn a_response_with_no_tag_is_an_error() {
        assert!(tag_from_release_json(b"{}").is_err());
        assert!(tag_from_release_json(b"not json").is_err());
    }

    #[test]
    fn what_changed_is_read_newest_first_from_a_compare() {
        let body = br#"{"status": "ahead", "commits": [
            {"sha": "a1", "commit": {"message": "Add rollout history\n\nwith details"}},
            {"sha": "b2", "commit": {"message": "Fix the header"}}]}"#;
        let compared = compared_from_json(body).unwrap();
        assert_eq!(compared.subjects, ["Fix the header", "Add rollout history"]);
        assert!(!compared.ahead);
    }

    #[test]
    fn a_build_past_the_release_is_ahead() {
        // GitHub compares from this build to the release's: "behind" means the release is.
        let compared = compared_from_json(br#"{"status": "behind", "commits": []}"#).unwrap();
        assert!(compared.ahead && compared.subjects.is_empty());
    }

    #[test]
    fn release_notes_keep_their_bullets_without_authors() {
        let body = "## What's Changed\n* Add debug containers by @someone in #4\n- Fix scaling\n\n**Full Changelog**: x";
        assert_eq!(release_notes(body), ["Add debug containers", "Fix scaling"]);
    }

    #[test]
    fn a_build_is_named_by_version_and_short_commit() {
        assert_eq!(build_name("0.1.0", "dd79c24f00aa"), "0.1.0 · dd79c24");
        assert_eq!(build_name("0.1.0", ""), "0.1.0");
    }

    #[test]
    fn version_strips_the_leading_v() {
        assert_eq!(version("v0.2.0"), "0.2.0");
    }

    #[test]
    fn a_matching_checksum_passes_and_a_wrong_one_does_not() {
        let bytes = b"the binary";
        let good = format!("{}  knav-x86_64-unknown-linux-musl", hex_sha256(bytes));
        assert!(verify(bytes, good.as_bytes()).is_ok());
        assert!(verify(bytes, b"0000000000000000000000000000000000000000000000000000000000000000  x").is_err());
    }

    #[test]
    fn the_running_binary_matches_only_its_own_checksum() {
        let exe = std::env::temp_dir().join(format!("knav-same-build-{}", std::process::id()));
        std::fs::write(&exe, b"build one").unwrap();
        let own = format!("{}  knav", hex_sha256(b"build one"));
        let other = format!("{}  knav", hex_sha256(b"build two"));
        assert!(same_build(&exe, own.as_bytes()));
        assert!(!same_build(&exe, other.as_bytes()));
        std::fs::remove_file(exe).unwrap();
    }

    #[test]
    fn a_homebrew_or_nix_path_refuses_to_self_update() {
        assert!(refuse_if_package_managed(std::path::Path::new("/opt/homebrew/Cellar/knav/0.1.0/bin/knav")).is_err());
        assert!(refuse_if_package_managed(std::path::Path::new("/nix/store/abc/bin/knav")).is_err());
        assert!(refuse_if_package_managed(std::path::Path::new("/home/me/.local/bin/knav")).is_ok());
    }

    #[test]
    fn every_target_the_workflow_builds_has_a_triple_and_others_do_not() {
        for (os, arch) in [("macos", "aarch64"), ("macos", "x86_64"), ("linux", "x86_64"), ("linux", "aarch64")] {
            assert!(triple_for(os, arch).is_some());
        }
        assert!(triple_for("windows", "x86_64").is_none());
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Against the real GitHub API, read-only: `cargo test live_update -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_latest_tag_parses() {
        let tag = latest_release().unwrap().tag;
        println!("latest release: {tag}");
        assert!(tag.starts_with('v'));
    }
}
