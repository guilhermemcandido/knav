//! The shape of an extension manifest (`extensions/*.toml`), and nothing else: a
//! manifest can only select from the templates this crate already implements,
//! never supply code of its own. See `Registry` for how these load.
//!
//! Extensions are read-only by design: a manifest can attach a category, icon
//! and detail view to CRD kinds the cluster already has, but has no mechanism
//! to patch, annotate, create or delete anything. `category`/`kinds` are wired
//! up (see `Catalog::extension_sections`); `view` renders in the object's
//! detail view (see `extensions::view_for` and `k8s::details::details`).
//! `icon` is still reserved, unused.
//!
//! `dashboard` is the same philosophy applied across several kinds at once —
//! a fixed, closed set of widgets (`count`/`tally`/`sum`/`list`) a manifest
//! picks from and points at fields, never code of its own (see
//! `extensions::dashboards::declarative`). It caps out short of anything
//! that needs a join across kinds (Karpenter's Node↔NodePool, say); those
//! stay hand-written Rust, bundled with knav rather than user-authored —
//! unless the join is worth writing as `wasm_dashboard` instead: a compiled
//! WASM component, sandboxed by the same "no code of its own" philosophy
//! taken to its limit — real Rust, but with zero imports (see
//! `wit/dashboard.wit`), so it can only transform the objects it's handed
//! into lines, never reach a client, a file, or the network. Mutually
//! exclusive with `dashboard` (one mechanism per extension) and, for now,
//! external manifests only — see `extensions::dashboards::wasm`.

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Manifest {
    pub extension: ExtensionMeta,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExtensionMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "kind", default)]
    pub kinds: Vec<ExtKind>,
    /// An aggregate dashboard, one category's worth of widgets across
    /// several of `kinds` at once — see `DashboardWidget`. Optional: most
    /// extensions are fine with just the category tile Overview already
    /// gives every kind for free.
    #[serde(rename = "dashboard", default)]
    pub dashboard: Vec<DashboardWidget>,
    /// A WASM component's filename, resolved against this manifest's own
    /// directory (see `Manifest::parse`'s validation and
    /// `extensions::Registry::load`'s path resolution) — the code-carrying
    /// alternative to `dashboard`'s declarative widgets, for a category-wide
    /// dashboard that needs real logic (a join across kinds, say) rather
    /// than one of the four fixed widget shapes.
    #[serde(default)]
    pub wasm_dashboard: Option<String>,
}

/// One CRD kind an extension attaches metadata to, matched by `group`+`kind`
/// against whatever the cluster actually has installed (see `k8s::CrdInfo`).
/// A kind whose CRD isn't installed simply contributes nothing, no error.
#[derive(Clone, Debug, Deserialize)]
pub struct ExtKind {
    pub group: String,
    pub kind: String,
    pub category: String,
    /// Reserved for a future icon override; unused today (falls back to the
    /// generic custom-resource icon).
    #[serde(default)]
    #[allow(dead_code)]
    pub icon: Option<String>,
    /// The detail-view content template shown for an object of this kind
    /// instead of the generic field dump (see `k8s::details::details`).
    #[serde(default)]
    pub view: Option<ViewTemplate>,
}

pub use crate::k8s::details::ViewTemplate;

/// One block of an extension's dashboard (see `extensions::dashboards`):
/// what it covers (`kind`, plus `extra_kinds` to fold more than one kind's
/// objects into the same widget — Issuer and ClusterIssuer read as one
/// "Issuers" tally, say) and how (`spec`). Declaration order is render order.
#[derive(Clone, Debug, Deserialize)]
pub struct DashboardWidget {
    pub kind: String,
    #[serde(default)]
    pub extra_kinds: Vec<String>,
    /// Overrides the heading this widget renders under; defaults to `kind`
    /// (`extra_kinds` joined in with " / ").
    #[serde(default)]
    pub label: Option<String>,
    #[serde(flatten)]
    pub spec: WidgetSpec,
}

/// A widget is one fixed shape, not a rendering instruction, the same
/// philosophy as `ViewTemplate`: the manifest supplies field paths, this
/// crate supplies the math and the drawing.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "widget", rename_all = "snake_case")]
pub enum WidgetSpec {
    /// Just "N Kind(s)" — for a kind with nothing else worth showing.
    Count,
    /// Buckets every object into True/False/Unknown by:
    /// - `"ready"`, shorthand for `condition:Ready`
    /// - `"condition:<Type>"` — `.status.conditions[type=Type].status`
    /// - `"field:<.dotted.path>"` — a plain boolean/string field read directly
    Tally { by: String },
    /// Sums one or more numeric fields across every object, `[label, path]`
    /// pairs like `KeyValues` — e.g. a PolicyReport's `.summary.pass`.
    Sum { fields: Vec<[String; 2]> },
    /// One row per object: `columns` are `[label, path]` pairs like
    /// `KeyValues`. `sort_by`, if given, orders ascending (numeric, an
    /// RFC3339 timestamp, or lexical — whichever the values actually are).
    /// A column whose path is also in `date_columns` renders as "in Nd" (or
    /// "Nd ago"), colour-coded by how soon, instead of the raw timestamp.
    List {
        #[serde(default)]
        sort_by: Option<String>,
        #[serde(default)]
        date_columns: Vec<String>,
        columns: Vec<[String; 2]>,
        #[serde(default = "default_list_limit")]
        limit: usize,
    },
}

fn default_list_limit() -> usize {
    20
}

#[derive(Debug)]
pub enum ParseError {
    Toml(toml::de::Error),
    /// A kind with no fields to key off.
    Invalid(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Toml(e) => write!(f, "{e}"),
            ParseError::Invalid(msg) => write!(f, "{msg}"),
        }
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, ParseError> {
        let manifest: Manifest = toml::from_str(text).map_err(ParseError::Toml)?;
        if manifest.extension.id.trim().is_empty() {
            return Err(ParseError::Invalid("extension.id can't be empty".into()));
        }
        for kind in &manifest.extension.kinds {
            if kind.kind.trim().is_empty() {
                return Err(ParseError::Invalid(format!("{}: a kind entry is missing its `kind`", manifest.extension.id)));
            }
        }
        for widget in &manifest.extension.dashboard {
            for kind in std::iter::once(&widget.kind).chain(&widget.extra_kinds) {
                if !manifest.extension.kinds.iter().any(|k| &k.kind == kind) {
                    return Err(ParseError::Invalid(format!("{}: dashboard widget references kind `{kind}`, which isn't in `extension.kind`", manifest.extension.id)));
                }
            }
            if let WidgetSpec::Tally { by } = &widget.spec
                && by != "ready"
                && !by.starts_with("condition:")
                && !by.starts_with("field:.")
            {
                return Err(ParseError::Invalid(format!("{}: dashboard tally `by` must be \"ready\", \"condition:<Type>\" or \"field:<.path>\", got \"{by}\"", manifest.extension.id)));
            }
        }
        if let Some(wasm) = &manifest.extension.wasm_dashboard {
            if wasm.trim().is_empty() {
                return Err(ParseError::Invalid(format!("{}: wasm_dashboard can't be empty", manifest.extension.id)));
            }
            if std::path::Path::new(wasm).components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir)) {
                return Err(ParseError::Invalid(format!("{}: wasm_dashboard must be a plain filename, not a path (\"{wasm}\")", manifest.extension.id)));
            }
            if !manifest.extension.dashboard.is_empty() {
                return Err(ParseError::Invalid(format!("{}: wasm_dashboard and dashboard are mutually exclusive — pick one", manifest.extension.id)));
            }
            let categories: std::collections::BTreeSet<&str> = manifest.extension.kinds.iter().map(|k| k.category.as_str()).collect();
            if categories.len() != 1 {
                return Err(ParseError::Invalid(format!("{}: wasm_dashboard needs every kind under one category, found {}", manifest.extension.id, categories.len())));
            }
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_manifest_parses() {
        let m = Manifest::parse(
            r#"
            [extension]
            id = "flux"
            name = "Flux"

            [[extension.kind]]
            group = "kustomize.toolkit.fluxcd.io"
            kind = "Kustomization"
            category = "GitOps"
            "#,
        )
        .unwrap();
        assert_eq!(m.extension.id, "flux");
        assert_eq!(m.extension.kinds.len(), 1);
        assert_eq!(m.extension.kinds[0].category, "GitOps");
    }

    #[test]
    fn a_view_template_parses_with_its_data_only() {
        let m = Manifest::parse(
            r#"
            [extension]
            id = "flux"
            name = "Flux"

            [[extension.kind]]
            group = "kustomize.toolkit.fluxcd.io"
            kind = "Kustomization"
            category = "GitOps"

            [extension.kind.view]
            template = "timeline"
            from = ".status.conditions"
            "#,
        )
        .unwrap();
        let kind = &m.extension.kinds[0];
        assert!(matches!(&kind.view, Some(ViewTemplate::Timeline { from }) if from == ".status.conditions"));
    }

    #[test]
    fn a_dashboard_widget_parses_and_defaults_its_limit() {
        let m = Manifest::parse(
            r#"
            [extension]
            id = "cert-manager"
            name = "cert-manager"

            [[extension.kind]]
            group = "cert-manager.io"
            kind = "Certificate"
            category = "cert-manager"

            [[extension.dashboard]]
            kind = "Certificate"
            widget = "list"
            sort_by = ".status.notAfter"
            date_columns = [".status.notAfter"]
            columns = [["Name", ".metadata.name"], ["Expires", ".status.notAfter"]]
            "#,
        )
        .unwrap();
        let widget = &m.extension.dashboard[0];
        assert_eq!(widget.kind, "Certificate");
        match &widget.spec {
            WidgetSpec::List { sort_by, columns, limit, .. } => {
                assert_eq!(sort_by.as_deref(), Some(".status.notAfter"));
                assert_eq!(columns.len(), 2);
                assert_eq!(*limit, 20);
            }
            other => panic!("expected a list widget, got {other:?}"),
        }
    }

    #[test]
    fn a_dashboard_widget_referencing_an_undeclared_kind_is_rejected() {
        let err = Manifest::parse(
            r#"
            [extension]
            id = "x"
            name = "x"

            [[extension.dashboard]]
            kind = "Certificate"
            widget = "count"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ParseError::Invalid(_)));
    }

    #[test]
    fn a_tally_with_a_malformed_by_is_rejected() {
        let err = Manifest::parse(
            r#"
            [extension]
            id = "x"
            name = "x"

            [[extension.kind]]
            group = "g"
            kind = "K"
            category = "C"

            [[extension.dashboard]]
            kind = "K"
            widget = "tally"
            by = "nonsense"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ParseError::Invalid(_)));
    }

    #[test]
    fn a_blank_id_is_rejected() {
        assert!(Manifest::parse("[extension]\nid = \"\"\nname = \"x\"\n").is_err());
    }

    #[test]
    fn a_wasm_dashboard_field_parses() {
        let m = Manifest::parse(
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
        assert_eq!(m.extension.wasm_dashboard.as_deref(), Some("dashboard.wasm"));
    }

    #[test]
    fn wasm_dashboard_and_declarative_dashboard_together_is_rejected() {
        let err = Manifest::parse(
            r#"
            [extension]
            id = "x"
            name = "x"
            wasm_dashboard = "dashboard.wasm"

            [[extension.kind]]
            group = "g"
            kind = "K"
            category = "C"

            [[extension.dashboard]]
            kind = "K"
            widget = "count"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ParseError::Invalid(_)));
    }

    #[test]
    fn wasm_dashboard_with_a_path_escaping_upward_is_rejected() {
        let err = Manifest::parse("[extension]\nid = \"x\"\nname = \"x\"\nwasm_dashboard = \"../dashboard.wasm\"\n").unwrap_err();
        assert!(matches!(err, ParseError::Invalid(_)));
    }

    #[test]
    fn wasm_dashboard_kinds_must_share_one_category() {
        let err = Manifest::parse(
            r#"
            [extension]
            id = "x"
            name = "x"
            wasm_dashboard = "dashboard.wasm"

            [[extension.kind]]
            group = "g"
            kind = "A"
            category = "One"

            [[extension.kind]]
            group = "g"
            kind = "B"
            category = "Two"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ParseError::Invalid(_)));
    }

    #[test]
    fn malformed_toml_is_a_parse_error_not_a_panic() {
        assert!(Manifest::parse("not valid toml [[[").is_err());
    }
}
