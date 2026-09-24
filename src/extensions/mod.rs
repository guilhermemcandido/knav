//! Third-party integrations (Flux, Argo CD, Helm, Karpenter, cert-manager, KEDA,
//! Prometheus Operator, Crossplane, Istio, Kyverno, OPA Gatekeeper) as data, not code: an
//! extension is a TOML manifest (see `manifest`) that attaches a category,
//! and eventually a view, to CRD kinds the cluster already has. Extensions
//! are read-only: knav is a viewer, not a controller, so a manifest has no
//! way to patch, annotate, create or delete anything — it can only pick from
//! the display templates this crate implements.
//!
//! Two sources, loaded the same way: the few bundled here (compiled into the
//! binary via `include_str!`, so they work with no install step) and
//! whatever a user has dropped under `~/.config/knav/extensions/*/manifest.toml`
//! (what `knav ext add <repo>` will populate later). Disabled by default;
//! Settings' Extensions tab turns them on, which just adds their id to
//! `config.toml`'s `extensions.enabled`.

pub mod manifest;

use manifest::{ExtKind, Manifest};

/// One loaded manifest, bundled or from disk, for the Extensions settings
/// tab: whether it parsed, and what it'd add if enabled.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub id: String,
    pub name: String,
    pub description: String,
    pub bundled: bool,
    /// `Some` when the file failed to parse; the id/name are then guessed
    /// from the filename so it still has something to show.
    pub error: Option<String>,
    pub kinds: Vec<ExtKind>,
}

/// Every loaded extension, and a lookup from an installed CRD's `(group,
/// kind)` to whichever enabled extension claims it.
#[derive(Default)]
pub struct Registry {
    pub loaded: Vec<Loaded>,
}

/// `(id, manifest text)` for every extension shipped with knav. Helm is
/// two things under one toggle: real releases, which aren't a CRD (they're
/// Secrets) so knav reads them natively (see `k8s::helm`), plus Rancher/k3s's
/// own declarative-install CRDs (`helm.cattle.io`) where a cluster has them —
/// an ordinary `[[extension.kind]]` match like any other extension.
const BUNDLED: &[(&str, &str)] = &[
    ("flux", include_str!("../../extensions/flux.toml")),
    ("argocd", include_str!("../../extensions/argocd.toml")),
    ("helm", include_str!("../../extensions/helm.toml")),
    ("karpenter", include_str!("../../extensions/karpenter.toml")),
    ("cert-manager", include_str!("../../extensions/cert-manager.toml")),
    ("keda", include_str!("../../extensions/keda.toml")),
    ("prometheus", include_str!("../../extensions/prometheus.toml")),
    ("crossplane", include_str!("../../extensions/crossplane.toml")),
    ("istio", include_str!("../../extensions/istio.toml")),
    ("kyverno", include_str!("../../extensions/kyverno.toml")),
    ("gatekeeper", include_str!("../../extensions/gatekeeper.toml")),
];

impl Registry {
    /// Loads every bundled extension plus anything under
    /// `~/.config/knav/extensions/*/manifest.toml`. Never fails: a bad
    /// manifest shows up with `error` set instead of stopping the others.
    pub fn load(config_dir: &std::path::Path) -> Registry {
        let mut loaded: Vec<Loaded> = BUNDLED.iter().map(|(id, text)| from_text(id, text, true)).collect();
        let user_dir = config_dir.join("extensions");
        if let Ok(entries) = std::fs::read_dir(&user_dir) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            for entry in dirs {
                let manifest_path = entry.path().join("manifest.toml");
                let Ok(text) = std::fs::read_to_string(&manifest_path) else { continue };
                let fallback_id = entry.file_name().to_string_lossy().into_owned();
                loaded.push(from_text(&fallback_id, &text, false));
            }
        }
        Registry { loaded }
    }

    /// The `ExtKind`s an enabled extension wants added to the Overview/sidebar,
    /// for kinds the cluster actually has (matched by the caller against its
    /// discovered CRDs).
    pub fn enabled_kinds<'a>(&'a self, enabled: &[String]) -> impl Iterator<Item = &'a ExtKind> + 'a {
        let enabled = enabled.to_vec();
        self.loaded.iter().filter(move |l| l.error.is_none() && enabled.iter().any(|e| e == &l.id)).flat_map(|l| l.kinds.iter())
    }
}

/// The Extensions tab's display order: `loaded` narrowed to whatever
/// fuzzy-matches `filter` on its name (everything, when `filter` is empty),
/// bundled ones first, alphabetical by name within each group. Indexes into
/// `loaded`, so both the tab's rows and a toggle's lookup of the actual
/// `Loaded` it acted on come from the same list.
pub fn visible_order(loaded: &[Loaded], filter: &str) -> Vec<usize> {
    let mut order: Vec<usize> = (0..loaded.len()).filter(|&i| filter.is_empty() || crate::startup::fuzzy::positions(filter, &loaded[i].name).is_some()).collect();
    order.sort_by_key(|&i| (!loaded[i].bundled, loaded[i].name.to_lowercase()));
    order
}

fn from_text(fallback_id: &str, text: &str, bundled: bool) -> Loaded {
    match Manifest::parse(text) {
        Ok(m) => Loaded { id: m.extension.id, name: m.extension.name, description: m.extension.description, bundled, error: None, kinds: m.extension.kinds },
        Err(e) => Loaded { id: fallback_id.to_string(), name: fallback_id.to_string(), description: String::new(), bundled, error: Some(e.to_string()), kinds: Vec::new() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_extension_parses_clean() {
        let dir = tempdir();
        let registry = Registry::load(dir.path());
        assert_eq!(registry.loaded.len(), BUNDLED.len());
        for loaded in &registry.loaded {
            assert!(loaded.error.is_none(), "{}: {:?}", loaded.id, loaded.error);
            assert!(!loaded.kinds.is_empty(), "{} declares no kinds", loaded.id);
        }
    }

    fn stub(id: &str, name: &str, bundled: bool) -> Loaded {
        Loaded { id: id.into(), name: name.into(), description: String::new(), bundled, error: None, kinds: Vec::new() }
    }

    #[test]
    fn bundled_sorts_first_then_external_each_alphabetical() {
        let loaded = vec![stub("z", "Zeta", true), stub("a", "Alpha External", false), stub("k", "Karpenter", true), stub("b", "Beta External", false)];
        let order = visible_order(&loaded, "");
        let names: Vec<&str> = order.iter().map(|&i| loaded[i].name.as_str()).collect();
        assert_eq!(names, ["Karpenter", "Zeta", "Alpha External", "Beta External"]);
    }

    #[test]
    fn a_filter_narrows_by_name_and_keeps_the_grouping() {
        let loaded = vec![stub("flux", "Flux", true), stub("argocd", "Argo CD", true), stub("mine", "My Argo Thing", false)];
        let order = visible_order(&loaded, "argo");
        let names: Vec<&str> = order.iter().map(|&i| loaded[i].name.as_str()).collect();
        assert_eq!(names, ["Argo CD", "My Argo Thing"], "matches by name regardless of bundled/external");
    }

    #[test]
    fn only_enabled_extensions_contribute_kinds() {
        let dir = tempdir();
        let registry = Registry::load(dir.path());
        assert_eq!(registry.enabled_kinds(&[]).count(), 0);
        let flux_only: Vec<String> = vec!["flux".into()];
        let kinds: Vec<&ExtKind> = registry.enabled_kinds(&flux_only).collect();
        assert!(kinds.iter().any(|k| k.group == "kustomize.toolkit.fluxcd.io" && k.kind == "Kustomization" && k.category == "GitOps"));
        assert!(!kinds.iter().any(|k| k.group == "argoproj.io"), "argocd isn't enabled");
    }

    #[test]
    fn a_user_extension_with_bad_toml_is_listed_with_an_error_not_dropped() {
        let dir = tempdir();
        let ext_dir = dir.path().join("extensions").join("broken");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.toml"), "not valid toml [[[").unwrap();
        let registry = Registry::load(dir.path());
        let broken = registry.loaded.iter().find(|l| l.id == "broken").expect("still listed");
        assert!(broken.error.is_some());
        assert!(!broken.bundled);
    }

    #[test]
    fn a_user_extension_that_parses_is_loaded_alongside_the_bundled_ones() {
        let dir = tempdir();
        let ext_dir = dir.path().join("extensions").join("mine");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.toml"),
            "[extension]\nid = \"mine\"\nname = \"Mine\"\n\n[[extension.kind]]\ngroup = \"example.com\"\nkind = \"Widget\"\ncategory = \"Widgets\"\n",
        )
        .unwrap();
        let registry = Registry::load(dir.path());
        assert!(registry.loaded.iter().any(|l| l.id == "mine" && l.error.is_none() && !l.bundled));
    }

    fn tempdir() -> tempfile_shim::TempDir {
        tempfile_shim::TempDir::new()
    }

    /// A tiny stand-in for the `tempfile` crate (not a dependency here): a
    /// directory under the system temp dir, removed when dropped.
    mod tempfile_shim {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);

        pub struct TempDir(std::path::PathBuf);
        impl TempDir {
            pub fn new() -> Self {
                let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
                let dir = std::env::temp_dir().join(format!("knav-ext-test-{}-{nonce}", std::process::id()));
                std::fs::create_dir_all(&dir).unwrap();
                TempDir(dir)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
