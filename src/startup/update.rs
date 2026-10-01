//! `knav update`: replaces this binary with the latest GitHub release when newer.
//! Uses `curl` rather than an HTTP client crate for this one command.

use anyhow::{Context as _, Result};

use std::io::Write;

use sha2::{Digest, Sha256};


const REPO: &str = "guilhermemcandido/knav";

pub(crate) fn run(auto_yes: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let latest = latest_tag()?;
    if version(&latest) == current {
        println!("knav {current} is up to date.");
        return Ok(());
    }
    println!("knav {current} -> {latest}");
    if !auto_yes && !confirm(&format!("Update to {latest}? [y/N] "))? {
        return Ok(());
    }

    let target = target_triple().with_context(|| format!("no prebuilt binary for {}/{}; reinstall with `cargo install --git https://github.com/{REPO}`", std::env::consts::OS, std::env::consts::ARCH))?;
    let exe = running_exe()?;
    refuse_if_package_managed(&exe)?;

    let bytes = download(&format!("https://github.com/{REPO}/releases/download/{latest}/knav-{target}"))?;
    let checksum = download(&format!("https://github.com/{REPO}/releases/download/{latest}/knav-{target}.sha256"))?;
    verify(&bytes, &checksum)?;
    replace_running_binary(&exe, &bytes)?;
    println!("Updated to {latest}. Run it again to use the new version.");
    Ok(())
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

/// The tag GitHub reports as the latest release, e.g. `v0.2.0`.
fn latest_tag() -> Result<String> {
    let body = download(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
    tag_from_release_json(&body)
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
        let tag = latest_tag().unwrap();
        println!("latest release: {tag}");
        assert!(tag.starts_with('v'));
    }
}
