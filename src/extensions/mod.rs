//! Third-party integrations (Flux, Argo CD, Helm, Karpenter, cert-manager, KEDA,
//! Prometheus, Crossplane, Istio, Kyverno, OPA Gatekeeper) as data, not code: an
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
//!
//! `dashboards` is the other half: a category-wide aggregate screen across
//! several of an extension's kinds at once (a tally, a sorted list, a sum —
//! see `manifest::DashboardWidget`), which the per-kind `view` templates
//! can't express. Most of these ARE just data: `[[extension.dashboard]]`
//! blocks in the same manifest, so `knav ext add` reaches them too, same as
//! `view`. The couple that genuinely need a join across kinds (Karpenter's
//! Node↔NodePool, GitOps's Flux+Argo CD combination) stay native Rust,
//! bundled and reviewed rather than user-authored — but even those are
//! reached through the same small trait as the declarative ones, so the
//! core app never has to know a dashboard's name to show it.

pub mod dashboards;
pub mod manifest;

use manifest::{DashboardWidget, ExtKind, Manifest};

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
    pub dashboard: Vec<DashboardWidget>,
    /// `manifest::ExtensionMeta::wasm_dashboard`, resolved to an absolute,
    /// canonicalized path already proven to sit inside this extension's own
    /// directory and under the size cap — never set for a bundled extension
    /// (see `resolve_wasm_dashboard`). Still just a path: nothing has read
    /// or compiled the file yet (see `dashboards::wasm`).
    pub wasm_dashboard: Option<std::path::PathBuf>,
}

/// Every loaded extension, and a lookup from an installed CRD's `(group,
/// kind)` to whichever enabled extension claims it.
#[derive(Default)]
pub struct Registry {
    pub loaded: Vec<Loaded>,
    /// Compiled WASM dashboards, keyed by extension id — built once here, at
    /// load time, so `dashboards::find` only ever borrows a ready-to-run
    /// module instead of recompiling one every frame. A `Loaded` whose
    /// `wasm_dashboard` failed to compile has no entry here; its `error` is
    /// set instead (see `load`), same "never stops the others" contract as
    /// a bad manifest.
    pub(crate) wasm: std::collections::HashMap<String, dashboards::wasm::WasmDashboardModule>,
    /// The one process-wide WASM engine every compiled module and every
    /// `dashboards::wasm::call` runs against. `None` only if building the
    /// engine itself failed — vanishingly unlikely, and handled the same way
    /// as any other extension failure: every `wasm_dashboard` extension is
    /// then just unusable, not a crash.
    pub(crate) wasm_engine: Option<wasmtime::Engine>,
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
        let mut loaded: Vec<Loaded> = BUNDLED.iter().map(|(id, text)| from_text(id, text, true, None)).collect();
        let user_dir = config_dir.join("extensions");
        if let Ok(entries) = std::fs::read_dir(&user_dir) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            for entry in dirs {
                let manifest_path = entry.path().join("manifest.toml");
                let Ok(text) = std::fs::read_to_string(&manifest_path) else { continue };
                let fallback_id = entry.file_name().to_string_lossy().into_owned();
                loaded.push(from_text(&fallback_id, &text, false, Some(&entry.path())));
            }
        }
        let wasm_engine = dashboards::wasm::engine().ok();
        let mut wasm = std::collections::HashMap::new();
        for l in &mut loaded {
            let Some(path) = l.wasm_dashboard.as_deref() else { continue };
            let compiled = wasm_engine.as_ref().ok_or_else(|| "the WASM engine failed to start".to_string()).and_then(|engine| dashboards::wasm::WasmDashboardModule::compile(engine, path));
            match compiled {
                Ok(module) => {
                    wasm.insert(l.id.clone(), module);
                }
                Err(e) => l.error = Some(e),
            }
        }
        Registry { loaded, wasm, wasm_engine }
    }

    /// The `ExtKind`s an enabled extension wants added to the Overview/sidebar,
    /// for kinds the cluster actually has (matched by the caller against its
    /// discovered CRDs).
    pub fn enabled_kinds<'a>(&'a self, enabled: &[String]) -> impl Iterator<Item = &'a ExtKind> + 'a {
        let enabled = enabled.to_vec();
        self.loaded.iter().filter(move |l| l.error.is_none() && enabled.iter().any(|e| e == &l.id)).flat_map(|l| l.kinds.iter())
    }

    /// The view template an enabled extension declares for `group`/`kind`,
    /// if any — what `k8s::details::details` renders for an object of that
    /// kind instead of the generic field dump.
    pub fn view_for<'a>(&'a self, enabled: &[String], group: &str, kind: &str) -> Option<&'a manifest::ViewTemplate> {
        self.enabled_kinds(enabled).find(|k| k.group == group && k.kind == kind).and_then(|k| k.view.as_ref())
    }

    /// Every dashboard widget any loaded (not necessarily enabled — see
    /// `dashboards::find`) manifest declares for `category`, with the
    /// `(group, kind)` each widget's own `kind`/`extra_kinds` resolve to,
    /// looked up against that same manifest's `kinds`. A widget whose kind
    /// isn't declared can't happen (`Manifest::parse` rejects it), so this
    /// silently skips nothing real.
    pub fn dashboard_widgets(&self, category: &str) -> Vec<(&DashboardWidget, Vec<(&str, &str)>)> {
        self.loaded
            .iter()
            .filter(|l| l.error.is_none())
            .flat_map(|l| l.dashboard.iter().map(move |w| (l, w)))
            .filter(|(l, w)| l.kinds.iter().any(|k| k.kind == w.kind && k.category == category))
            .map(|(l, w)| {
                let resolve = |kind: &str| l.kinds.iter().find(|k| k.kind == kind).map(|k| (k.group.as_str(), k.kind.as_str()));
                let sources = std::iter::once(w.kind.as_str()).chain(w.extra_kinds.iter().map(String::as_str)).filter_map(resolve).collect();
                (w, sources)
            })
            .collect()
    }

    /// Every category with at least one dashboard widget or a working
    /// `wasm_dashboard` among the loaded manifests, regardless of enabled —
    /// same "reachable by `:category` regardless of the toggle" precedent
    /// `HelmReleases` already set.
    pub fn dashboard_categories(&self) -> Vec<&str> {
        let mut categories: Vec<&str> = self
            .loaded
            .iter()
            .filter(|l| l.error.is_none())
            .flat_map(|l| {
                let widgets = l.dashboard.iter().filter_map(move |w| l.kinds.iter().find(|k| k.kind == w.kind).map(|k| k.category.as_str()));
                let wasm = (l.wasm_dashboard.is_some() && self.wasm.contains_key(&l.id)).then(|| l.kinds.first().map(|k| k.category.as_str())).flatten();
                widgets.chain(wasm)
            })
            .collect();
        categories.sort_unstable();
        categories.dedup();
        categories
    }
}

/// The Extensions tab's display order: `loaded` narrowed to whatever
/// fuzzy-matches `filter` on its name (everything, when `filter` is empty),
/// bundled ones first, alphabetical by name within each group. Indexes into
/// `loaded`, so both the tab's rows and a toggle's lookup of the actual
/// `Loaded` it acted on come from the same list.
pub fn visible_order(loaded: &[Loaded], filter: &str) -> Vec<usize> {
    let mut order: Vec<usize> = (0..loaded.len()).filter(|&i| filter.is_empty() || crate::util::fuzzy::positions(filter, &loaded[i].name).is_some()).collect();
    order.sort_by_key(|&i| (!loaded[i].bundled, loaded[i].name.to_lowercase()));
    order
}

/// `dir` is this extension's own directory (external only — `None` for a
/// bundled one, which has no directory of its own to resolve a relative
/// `wasm_dashboard` path against).
fn from_text(fallback_id: &str, text: &str, bundled: bool, dir: Option<&std::path::Path>) -> Loaded {
    let empty = || Loaded { id: fallback_id.to_string(), name: fallback_id.to_string(), description: String::new(), bundled, error: None, kinds: Vec::new(), dashboard: Vec::new(), wasm_dashboard: None };
    match Manifest::parse(text) {
        Ok(m) => match resolve_wasm_dashboard(m.extension.wasm_dashboard.as_deref(), dir) {
            Ok(wasm_dashboard) => Loaded { id: m.extension.id, name: m.extension.name, description: m.extension.description, kinds: m.extension.kinds, dashboard: m.extension.dashboard, wasm_dashboard, ..empty() },
            Err(e) => Loaded { id: m.extension.id, name: m.extension.name, description: m.extension.description, error: Some(e), ..empty() },
        },
        Err(e) => Loaded { error: Some(e.to_string()), ..empty() },
    }
}

/// The most a `wasm_dashboard` file can be — generous for real dashboard
/// logic, small enough that a mistaken or hostile file can't be read wholly
/// into memory just by sitting in the extensions directory.
const WASM_DASHBOARD_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// `manifest::ExtensionMeta::wasm_dashboard`, turned into an absolute path
/// — or an error, same "never panics, becomes `Loaded.error`" contract as
/// everything else here. Proves the path stays inside the extension's own
/// directory (a `..` component is already rejected at parse time; this also
/// catches a symlink that resolves outside it) and under
/// `WASM_DASHBOARD_MAX_BYTES`, but doesn't read the file's contents — that's
/// `dashboards::wasm::WasmDashboardModule::compile`'s job.
fn resolve_wasm_dashboard(name: Option<&str>, dir: Option<&std::path::Path>) -> Result<Option<std::path::PathBuf>, String> {
    let Some(name) = name else { return Ok(None) };
    let Some(dir) = dir else {
        return Err("wasm_dashboard isn't available to bundled extensions".to_string());
    };
    let canonical_dir = std::fs::canonicalize(dir).map_err(|e| format!("resolving the extension's directory: {e}"))?;
    let canonical = std::fs::canonicalize(dir.join(name)).map_err(|e| format!("wasm_dashboard \"{name}\": {e}"))?;
    if !canonical.starts_with(&canonical_dir) {
        return Err(format!("wasm_dashboard \"{name}\" resolves outside the extension's own directory"));
    }
    let size = std::fs::metadata(&canonical).map(|m| m.len()).unwrap_or(0);
    if size > WASM_DASHBOARD_MAX_BYTES {
        return Err(format!("wasm_dashboard \"{name}\" is {size} bytes, over the {WASM_DASHBOARD_MAX_BYTES}-byte limit"));
    }
    Ok(Some(canonical))
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
        Loaded { id: id.into(), name: name.into(), description: String::new(), bundled, error: None, kinds: Vec::new(), dashboard: Vec::new(), wasm_dashboard: None }
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

    #[test]
    fn a_bundled_extension_declaring_wasm_dashboard_is_impossible_by_construction() {
        // Bundled manifests have no directory of their own for a relative
        // wasm_dashboard path to resolve against — proven directly against
        // `from_text`, the same function `BUNDLED` loads through, with
        // `dir: None` exactly as `Registry::load` passes for every bundled
        // entry.
        let loaded = from_text("x", "[extension]\nid = \"x\"\nname = \"x\"\nwasm_dashboard = \"dashboard.wasm\"\n", true, None);
        assert!(loaded.error.is_some());
        assert!(loaded.wasm_dashboard.is_none());
    }

    #[test]
    fn a_wasm_dashboard_referencing_a_missing_file_is_a_load_error_not_a_panic() {
        let dir = tempdir();
        let ext_dir = dir.path().join("extensions").join("ghost");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.toml"),
            "[extension]\nid = \"ghost\"\nname = \"Ghost\"\nwasm_dashboard = \"dashboard.wasm\"\n\n[[extension.kind]]\ngroup = \"example.com\"\nkind = \"Widget\"\ncategory = \"Widgets\"\n",
        )
        .unwrap();
        let registry = Registry::load(dir.path());
        let ghost = registry.loaded.iter().find(|l| l.id == "ghost").expect("still listed");
        assert!(ghost.error.is_some(), "the wasm file doesn't exist, so this should be an error, not a panic");
    }

    #[test]
    fn a_wasm_dashboard_next_to_its_manifest_resolves_to_an_absolute_path() {
        // The bytes here aren't a real component — proving path resolution
        // doesn't require a real one. `dashboards::wasm`'s own tests (with a
        // real compiled fixture) cover compilation and execution; this is
        // just "did the path land where it should", which happens before
        // compilation is even attempted (see `resolve_wasm_dashboard`).
        let dir = tempdir();
        let ext_dir = dir.path().join("extensions").join("widgetco");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("dashboard.wasm"), b"not really wasm, just needs to exist for this test").unwrap();
        std::fs::write(
            ext_dir.join("manifest.toml"),
            "[extension]\nid = \"widgetco\"\nname = \"Widget Co\"\nwasm_dashboard = \"dashboard.wasm\"\n\n[[extension.kind]]\ngroup = \"example.com\"\nkind = \"Widget\"\ncategory = \"Widgets\"\n",
        )
        .unwrap();
        let registry = Registry::load(dir.path());
        let widgetco = registry.loaded.iter().find(|l| l.id == "widgetco").expect("still listed");
        let resolved = widgetco.wasm_dashboard.as_ref().expect("resolved");
        assert!(resolved.is_absolute());
        assert!(resolved.starts_with(std::fs::canonicalize(&ext_dir).unwrap()));
    }

    #[test]
    fn a_wasm_dashboard_that_isnt_really_wasm_fails_to_compile_without_panicking() {
        let dir = tempdir();
        let ext_dir = dir.path().join("extensions").join("fake");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("dashboard.wasm"), b"not really wasm").unwrap();
        std::fs::write(
            ext_dir.join("manifest.toml"),
            "[extension]\nid = \"fake\"\nname = \"Fake\"\nwasm_dashboard = \"dashboard.wasm\"\n\n[[extension.kind]]\ngroup = \"example.com\"\nkind = \"Widget\"\ncategory = \"Widgets\"\n",
        )
        .unwrap();
        let registry = Registry::load(dir.path());
        let fake = registry.loaded.iter().find(|l| l.id == "fake").expect("still listed");
        assert!(fake.error.is_some(), "garbage bytes should fail to compile, not panic");
        assert!(!registry.wasm.contains_key("fake"));
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
