//! Native dashboards: one aggregate screen per extension category (Karpenter,
//! GitOps, cert-manager, ...), joining and summarizing several CRD kinds at
//! once. Most of these are entirely data — a manifest's own
//! `[[extension.dashboard]]` widgets (see `declarative` and
//! `manifest::DashboardWidget`), interpreted the same way its `view`
//! templates are, so `knav ext add` reaches them too. A manifest that needs
//! real logic instead of a fixed widget shape can ship `wasm_dashboard` —
//! real Rust, compiled to a WASM component with zero imports (see `wasm` and
//! `wit/dashboard.wit`), sandboxed so it can only turn the objects it's
//! handed into lines, nothing else. The couple that need a join across
//! *multiple extensions'* kinds (`karpenter`, `gitops`) still stay
//! hand-written and bundled rather than user-authored. All three are reached
//! through the exact same [`Dashboard`] trait or [`Found`] — `Catalog`/
//! `derive`/`ui` never know which kind of dashboard they're holding.

mod context;
mod declarative;
mod gitops;
mod karpenter;
/// `pub(super)`, not private: `extensions::Registry` (this module's parent)
/// compiles and stores `wasm::WasmDashboardModule`s itself, so it needs to
/// name the type and call `wasm::engine`/`wasm::WasmDashboardModule::compile`
/// directly — the narrowest visibility that allows that one caller without
/// opening `wasm`'s internals to the rest of the crate.
pub(super) mod wasm;

use ratatui::text::Line;

use crate::extensions::Registry;

pub use context::DashboardContext;

/// One dashboard, native or declarative alike. `category` must match the
/// `category` its extension's manifest gives the CRD kinds it covers (that's
/// how the Overview tile that opens it, and `ResourceKind::ExtensionDashboard`,
/// find it again).
pub trait Dashboard {
    fn category(&self) -> &str;
    /// The box title. Owned, not `&'static str`: a declarative dashboard's
    /// comes from a manifest, which can be a third party's (`knav ext add`),
    /// not a compile-time constant.
    fn title(&self) -> String;
    fn lines(&self, ctx: &mut DashboardContext) -> Vec<Line<'static>>;
}

/// The dashboards too complex for the declarative schema — a real join
/// across kinds, or (GitOps) across two different extensions' worth of
/// kinds. Checked before the manifests, so a user's own
/// `[[extension.dashboard]]` can never shadow one of these.
const NATIVE: &[&dyn Dashboard] = &[&karpenter::Karpenter, &gitops::GitOps];

/// What `find` returns: one of `NATIVE` (a `'static` reference, nothing to
/// own), a dashboard built fresh from a manifest's widgets, or a compiled
/// WASM module. All three own what they need rather than borrowing
/// `Registry`: `derive.rs` needs `&mut Catalog` (inside `catalog.extensions`
/// itself) to build the `DashboardContext` a call to `lines` takes, so a
/// `Found` borrowed from `&catalog.extensions` would conflict with that —
/// `Engine`/`Component` clone cheaply (both `Arc`-backed under the hood),
/// so `Wasm` just takes its own handle instead of fighting the borrow
/// checker over it.
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

/// The dashboard for `category`, if any: one of `NATIVE`, else whichever
/// loaded (bundled or `knav ext add`-installed) manifest declares a kind
/// under this category first — regardless of whether that extension is
/// currently enabled, the same "reachable directly, toggle or not"
/// precedent `HelmReleases` already set. That manifest is the single source
/// for both the title and which of `wasm_dashboard`/`dashboard` runs
/// (`Manifest::parse` already guarantees a manifest never sets both), so two
/// manifests sharing a category can't race on which mechanism wins — the
/// first-loaded one's choice always does.
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

/// Every category with a dashboard right now: `NATIVE`'s, plus every
/// category any loaded manifest declares `[[extension.dashboard]]` widgets
/// for.
pub fn categories(registry: &Registry) -> Vec<&'static str> {
    let mut categories: Vec<&'static str> = NATIVE.iter().map(|d| {
        // `NATIVE` entries' `category()` really is `&'static str` under the
        // hood (a Rust literal); the trait just can't say so (a declarative
        // one's isn't). Re-resolving through `static_str` costs one leak
        // check, not a new allocation, since `intern` below dedupes by value.
        static_str(d.category())
    }).collect();
    for category in registry.dashboard_categories() {
        if !categories.contains(&category) {
            categories.push(static_str(category));
        }
    }
    categories
}

/// Leaks a category name into a `&'static str` the first time it's seen —
/// the same trade `Catalog::extension_sections` already makes for the exact
/// same reason: a manifest's category is a runtime `String`, but
/// `ResourceKind::ExtensionDashboard` needs a cheap `Copy` payload. Bounded:
/// there are only ever as many distinct category names as loaded manifests
/// declare, a handful for the life of the process.
fn static_str(s: &str) -> &'static str {
    crate::k8s::leak(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal stand-in for what `knav ext add <repo>` (not built yet — see
    /// `Registry`'s docs) or a user cloning a repo by hand would leave
    /// behind: a `manifest.toml` under `<config_dir>/extensions/<id>/`.
    /// `Registry::load` doesn't care how it got there, so this is a faithful
    /// test of that path without needing the installer itself.
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

        // Never added to `extensions.enabled` — same "reachable directly
        // regardless of the toggle" precedent `HelmReleases` already set.
        let found = find("WidgetDash", &registry).expect("a manifest's own dashboard widgets should be discoverable without enabling it first");
        assert_eq!(found.title(), "Widget Co");
        assert!(matches!(found, Found::Declarative(_)));
    }

    /// The same real compiled component `wasm`'s own tests exercise, reused
    /// here to prove `find()` resolves a dropped-in-by-hand `wasm_dashboard`
    /// manifest the same way it already does a declarative one — a category
    /// with a `[[extension.dashboard]]` (fully declarative), and one with
    /// `wasm_dashboard` set, look identical from `derive.rs`'s point of view.
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
        assert!(found.contains(&"cert-manager"), "declarative, from the bundled manifest");
        assert!(found.contains(&"Kyverno"), "declarative, from the bundled manifest");
    }

    /// A tiny stand-in for the `tempfile` crate (not a dependency here): a
    /// directory under the system temp dir, removed when dropped.
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
