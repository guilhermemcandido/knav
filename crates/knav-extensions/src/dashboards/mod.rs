//! Category dashboards. Most are a manifest's declarative widgets, some are a
//! sandboxed WASM component, and two (Karpenter, GitOps) are native Rust.
//! All are reached through `Found`, so callers never know which kind they hold.

mod context;
mod declarative;
mod gitops;
mod karpenter;
pub(super) mod wasm;

use ratatui::text::Line;

use crate::Registry;

pub use context::DashboardContext;

/// One native dashboard. `category` must match its extension's category, which is
/// how the Overview tile finds it.
pub trait Dashboard {
    fn category(&self) -> &str;
    /// The box title. Owned, since a declarative one comes from a manifest.
    fn title(&self) -> String;
    fn lines(&self, ctx: &mut DashboardContext) -> Vec<Line<'static>>;
}

/// Dashboards that need a real join across kinds. Checked before the manifests,
/// so a manifest can't shadow one.
const NATIVE: &[&dyn Dashboard] = &[&karpenter::Karpenter, &gitops::GitOps];

/// A found dashboard. Each owns what it needs (the engine and component are cheap
/// `Arc` clones), so the caller can lend `&mut Catalog` to it.
pub enum Found {
    Native(&'static dyn Dashboard),
    Declarative(declarative::DeclarativeDashboard),
    Wasm { title: String, engine: wasmtime::Engine, module: wasm::WasmDashboardModule, kinds: Vec<(String, String)> },
}

impl Found {
    pub fn title(&self) -> String {
        match self {
            Found::Native(d) => d.title(),
            Found::Declarative(d) => d.title(),
            Found::Wasm { title, .. } => title.clone(),
        }
    }

    pub fn lines(&self, ctx: &mut DashboardContext) -> Vec<Line<'static>> {
        match self {
            Found::Native(d) => d.lines(ctx),
            Found::Declarative(d) => d.lines(ctx),
            Found::Wasm { engine, module, kinds, .. } => wasm::call(engine, module, kinds, ctx),
        }
    }
}

/// The dashboard for `category`: a native one, else the first loaded manifest with a
/// kind in it, enabled or not. That manifest alone decides the title and mechanism.
pub fn find(category: &str, registry: &Registry) -> Option<Found> {
    if let Some(d) = NATIVE.iter().find(|d| d.category() == category) {
        return Some(Found::Native(*d));
    }
    let owner = registry.loaded.iter().find(|l| l.error.is_none() && l.kinds.iter().any(|k| k.category == category))?;
    if owner.wasm_dashboard.is_some() {
        let module = registry.wasm.get(&owner.id)?.clone();
        let engine = registry.wasm_engine.as_ref()?.clone();
        let kinds = owner.kinds.iter().filter(|k| k.category == category).map(|k| (k.group.clone(), k.kind.clone())).collect();
        return Some(Found::Wasm { title: owner.name.clone(), engine, module, kinds });
    }
    let widgets: Vec<_> = registry.dashboard_widgets(category).into_iter().map(|(w, sources)| (w.clone(), sources.into_iter().map(|(g, k)| (g.to_string(), k.to_string())).collect())).collect();
    if widgets.is_empty() {
        return None;
    }
    Some(Found::Declarative(declarative::DeclarativeDashboard { title: owner.name.clone(), category: category.to_string(), widgets }))
}

/// Every category with a dashboard: the native ones plus every manifest's.
pub fn categories(registry: &Registry) -> Vec<&'static str> {
    let mut categories: Vec<&'static str> = NATIVE.iter().map(|d| {
        // A native category is already `'static`; the trait just can't say so.
        static_str(d.category())
    }).collect();
    for category in registry.dashboard_categories() {
        if !categories.contains(&category) {
            categories.push(static_str(category));
        }
    }
    categories
}

/// Leaks a category name once, since `ResourceKind::ExtensionDashboard` needs a
/// `&'static str`. Bounded by the number of distinct categories.
fn static_str(s: &str) -> &'static str {
    knav_k8s::leak(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Leaves a `manifest.toml` under `<config_dir>/extensions/<id>/`, like cloning
    /// an extension by hand.
    fn drop_local_extension(config_dir: &std::path::Path, id: &str, manifest: &str) {
        let ext_dir = config_dir.join("extensions").join(id);
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.toml"), manifest).unwrap();
    }

    #[test]
    fn a_dashboard_dropped_in_by_hand_is_found_by_its_category_never_enabled() {
        let dir = tempdir();
        drop_local_extension(
            dir.path(),
            "widgetco",
            r#"
            [extension]
            id = "widgetco"
            name = "Widget Co"

            [[extension.kind]]
            group = "widgets.example.com"
            kind = "Widget"
            category = "WidgetDash"

            [[extension.dashboard]]
            kind = "Widget"
            widget = "count"
            "#,
        );
        let registry = Registry::load(dir.path());
        assert!(registry.loaded.iter().any(|l| l.id == "widgetco" && l.error.is_none()), "the dropped manifest should load cleanly");

        // Never enabled: a dashboard is reachable either way.
        let found = find("WidgetDash", &registry).expect("a manifest's own dashboard widgets should be discoverable without enabling it first");
        assert_eq!(found.title(), "Widget Co");
        assert!(matches!(found, Found::Declarative(_)));
    }

    /// The compiled demo component, to show `find` resolves a WASM dashboard like a
    /// declarative one.
    const DEMO_WASM: &[u8] = include_bytes!("testdata/demo.wasm");

    #[test]
    fn a_wasm_backed_dashboard_dropped_in_by_hand_is_found_by_its_category() {
        let dir = tempdir();
        let ext_dir = dir.path().join("extensions").join("widgetco");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("dashboard.wasm"), DEMO_WASM).unwrap();
        std::fs::write(
            ext_dir.join("manifest.toml"),
            r#"
            [extension]
            id = "widgetco"
            name = "Widget Co"
            wasm_dashboard = "dashboard.wasm"

            [[extension.kind]]
            group = "widgets.example.com"
            kind = "Widget"
            category = "WidgetDash"
            "#,
        )
        .unwrap();
        let registry = Registry::load(dir.path());
        let loaded = registry.loaded.iter().find(|l| l.id == "widgetco").expect("still listed");
        assert!(loaded.error.is_none(), "{:?}", loaded.error);

        let found = find("WidgetDash", &registry).expect("a manifest's own wasm_dashboard should be discoverable without enabling it first");
        assert_eq!(found.title(), "Widget Co");
        assert!(matches!(found, Found::Wasm { .. }));
    }

    #[test]
    fn a_dropped_manifest_with_no_dashboard_widgets_contributes_no_category() {
        let dir = tempdir();
        drop_local_extension(
            dir.path(),
            "plain",
            r#"
            [extension]
            id = "plain"
            name = "Plain"

            [[extension.kind]]
            group = "plain.example.com"
            kind = "Thing"
            category = "PlainThings"
            "#,
        );
        let registry = Registry::load(dir.path());
        assert!(find("PlainThings", &registry).is_none(), "a category with no dashboard widgets has no dashboard, just the ordinary kind tile");
    }

    #[test]
    fn categories_lists_native_and_every_bundled_declarative_dashboard() {
        let dir = tempdir();
        let registry = Registry::load(dir.path()); // no user dir: bundled manifests only
        let found = categories(&registry);
        assert!(found.contains(&"Karpenter"), "native");
        assert!(found.contains(&"GitOps"), "native");
        assert!(found.contains(&"Cert-Manager"), "declarative, from the bundled manifest");
        assert!(found.contains(&"Kyverno"), "declarative, from the bundled manifest");
    }

    /// A temporary directory, removed when dropped.
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir() -> TempDir {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("knav-dashboards-test-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}
