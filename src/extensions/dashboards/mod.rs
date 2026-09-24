//! Native dashboards: one aggregate screen per extension category (Karpenter,
//! GitOps, cert-manager, ...), joining and summarizing several CRD kinds at
//! once. Most of these are entirely data — a manifest's own
//! `[[extension.dashboard]]` widgets (see `declarative` and
//! `manifest::DashboardWidget`), interpreted the same way its `view`
//! templates are, so `knav ext add` reaches them too. The couple that
//! genuinely need a join across kinds (`karpenter`, `gitops`) are hand-written
//! Rust instead, bundled and reviewed rather than user-authored, but reached
//! through the exact same [`Dashboard`] trait — `Catalog`/`derive`/`ui` never
//! know which kind of dashboard they're holding.

mod context;
mod declarative;
mod gitops;
mod karpenter;

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
/// own) or a dashboard built fresh from a manifest's widgets (owns its own
/// data, since it's assembled on the spot from `Registry`, not part of it).
pub enum Found {
    Native(&'static dyn Dashboard),
    Declarative(declarative::DeclarativeDashboard),
}

impl Found {
    pub fn title(&self) -> String {
        match self {
            Found::Native(d) => d.title(),
            Found::Declarative(d) => d.title(),
        }
    }

    pub fn lines(&self, ctx: &mut DashboardContext) -> Vec<Line<'static>> {
        match self {
            Found::Native(d) => d.lines(ctx),
            Found::Declarative(d) => d.lines(ctx),
        }
    }
}

/// The dashboard for `category`, if any: one of `NATIVE`, else built fresh
/// from whatever loaded (bundled or `knav ext add`-installed) manifest
/// declares `[[extension.dashboard]]` widgets for it — regardless of
/// whether that extension is currently enabled, the same "reachable
/// directly, toggle or not" precedent `HelmReleases` already set.
pub fn find(category: &str, registry: &Registry) -> Option<Found> {
    if let Some(d) = NATIVE.iter().find(|d| d.category() == category) {
        return Some(Found::Native(*d));
    }
    let widgets: Vec<_> = registry.dashboard_widgets(category).into_iter().map(|(w, sources)| (w.clone(), sources.into_iter().map(|(g, k)| (g.to_string(), k.to_string())).collect())).collect();
    if widgets.is_empty() {
        return None;
    }
    let title = registry.loaded.iter().find(|l| l.kinds.iter().any(|k| k.category == category)).map(|l| l.name.clone()).unwrap_or_else(|| category.to_string());
    Some(Found::Declarative(declarative::DeclarativeDashboard { title, category: category.to_string(), widgets }))
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
