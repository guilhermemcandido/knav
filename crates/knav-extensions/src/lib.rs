//! Read-only integrations (Flux, Argo CD, cert-manager, ...) described as TOML manifests:
//! the bundled ones and any under `~/.config/knav/extensions/*/manifest.toml`.
//! They are off by default; the Extensions screen (`E`) turns them on.

pub mod dashboards;
pub mod manifest;

use manifest::{DashboardWidget, ExtKind, Manifest};

/// One loaded manifest, bundled or from disk: whether it parsed, and what it adds.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub id: String,
    pub name: String,
    pub description: String,
    pub bundled: bool,
    /// Set when the manifest failed to load; the id and name then come from the filename.
    pub error: Option<String>,
    pub kinds: Vec<ExtKind>,
    pub dashboard: Vec<DashboardWidget>,
    /// The WASM dashboard's path, checked to sit inside the extension's own directory
    /// and under the size cap. Never set for a bundled extension.
    pub wasm_dashboard: Option<std::path::PathBuf>,
}

/// Every loaded extension, with their compiled WASM dashboards.
#[derive(Default)]
pub struct Registry {
    pub loaded: Vec<Loaded>,
    /// Compiled WASM dashboards by extension id, built once at load time. One that failed
    /// to compile has no entry and its extension's `error` set instead.
    pub wasm: std::collections::HashMap<String, dashboards::wasm::WasmDashboardModule>,
    /// The one WASM engine every module runs on. `None` if it failed to build, which
    /// leaves WASM dashboards unusable without failing anything else.
    pub wasm_engine: Option<wasmtime::Engine>,
}

/// Every extension shipped with knav, as `(id, manifest text)`. Helm releases are
/// Secrets, read natively by `knav_k8s::helm`; the manifest adds k3s's HelmChart CRDs.
const BUNDLED: &[(&str, &str)] = &[
    ("flux", include_str!("../bundled/flux.toml")),
    ("argocd", include_str!("../bundled/argocd.toml")),
    ("helm", include_str!("../bundled/helm.toml")),
    ("karpenter", include_str!("../bundled/karpenter.toml")),
    ("cert-manager", include_str!("../bundled/cert-manager.toml")),
    ("keda", include_str!("../bundled/keda.toml")),
    ("prometheus", include_str!("../bundled/prometheus.toml")),
    ("crossplane", include_str!("../bundled/crossplane.toml")),
    ("istio", include_str!("../bundled/istio.toml")),
    ("kyverno", include_str!("../bundled/kyverno.toml")),
    ("gatekeeper", include_str!("../bundled/gatekeeper.toml")),
];

impl Registry {
    /// Loads the bundled extensions and every external manifest. Never fails: a bad
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

    /// The plain-data view of every working extension that `Catalog` keeps.
    pub fn index(&self) -> knav_k8s::catalog::ExtensionIndex {
        let kinds = self
            .loaded
            .iter()
            .filter(|l| l.error.is_none())
            .flat_map(|l| l.kinds.iter().map(move |k| knav_k8s::catalog::IndexedKind { extension: l.id.clone(), group: k.group.clone(), kind: k.kind.clone(), category: k.category.clone(), view: k.view.clone() }))
            .collect();
        knav_k8s::catalog::ExtensionIndex { kinds, dashboards: dashboards::categories(self) }
    }

    /// Every dashboard widget the loaded manifests declare for `category`, enabled or
    /// not, with the `(group, kind)` pairs each widget reads.
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

    /// Every category with a dashboard widget or a working WASM dashboard, enabled or
    /// not, so `:category` reaches it either way.
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

/// The Extensions screen's order, as indexes into `loaded`: fuzzy-matched on name,
/// bundled first, then alphabetical.
pub fn visible_order(loaded: &[Loaded], filter: &str) -> Vec<usize> {
    let mut order: Vec<usize> = (0..loaded.len()).filter(|&i| filter.is_empty() || knav_common::util::fuzzy::positions(filter, &loaded[i].name).is_some()).collect();
    order.sort_by_key(|&i| (!loaded[i].bundled, loaded[i].name.to_lowercase()));
    order
}

/// `dir` is an external extension's own directory; `None` for a bundled one.
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

/// The largest WASM dashboard accepted, so a stray huge file isn't read into memory.
const WASM_DASHBOARD_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The WASM dashboard's absolute path, checked to stay inside the extension's directory
/// (symlinks included) and under `WASM_DASHBOARD_MAX_BYTES`. Doesn't read the file.
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
        let index = registry.index();
        assert_eq!(index.enabled(&[]).count(), 0);
        let flux_only: Vec<String> = vec!["flux".into()];
        let kinds: Vec<_> = index.enabled(&flux_only).collect();
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
        // Bundled manifests have no directory to resolve a WASM path against.
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
        // Not a real component: this only checks where the path lands, before compiling.
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

    /// A temporary directory, removed when dropped, so tests need no `tempfile` crate.
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
