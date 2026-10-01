//! The shape of an extension manifest. A manifest picks from templates and widgets
//! implemented here and points them at fields, so it can't run code or change anything.
//! A WASM dashboard is the one exception: code, but sandboxed with no imports.

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
    /// A dashboard of widgets across the extension's kinds. Optional.
    #[serde(rename = "dashboard", default)]
    pub dashboard: Vec<DashboardWidget>,
    /// A WASM component's filename in the manifest's own directory, for a dashboard
    /// that needs real logic. Can't be combined with `dashboard`.
    #[serde(default)]
    pub wasm_dashboard: Option<String>,
}

/// One CRD kind an extension adds, matched by group and kind. A kind whose CRD isn't
/// installed adds nothing.
#[derive(Clone, Debug, Deserialize)]
pub struct ExtKind {
    pub group: String,
    pub kind: String,
    pub category: String,
    /// Reserved for an icon override; unused.
    #[serde(default)]
    #[allow(dead_code)]
    pub icon: Option<String>,
    /// What the details show for this kind instead of the generic summary.
    #[serde(default)]
    pub view: Option<ViewTemplate>,
}

pub use knav_k8s::details::ViewTemplate;

/// One dashboard widget: the kind it covers, plus `extra_kinds` to fold others into it
/// (Issuer and ClusterIssuer as one tally). Declaration order is render order.
#[derive(Clone, Debug, Deserialize)]
pub struct DashboardWidget {
    pub kind: String,
    #[serde(default)]
    pub extra_kinds: Vec<String>,
    /// The widget's heading; defaults to its kinds joined with " / ".
    #[serde(default)]
    pub label: Option<String>,
    #[serde(flatten)]
    pub spec: WidgetSpec,
}

/// A widget's fixed shape. The manifest supplies field paths; the math and drawing are here.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "widget", rename_all = "snake_case")]
pub enum WidgetSpec {
    /// Just "N Kinds".
    Count,
    /// Buckets objects into True, False and Unknown by `ready`, `condition:<Type>` or
    /// `field:<.path>`.
    Tally { by: String },
    /// Sums numeric fields across every object, as `[label, path]` pairs.
    Sum { fields: Vec<[String; 2]> },
    /// One row per object from `[label, path]` columns, optionally sorted by `sort_by`.
    /// Paths in `date_columns` show as "in Nd" or "Nd ago", coloured by how soon.
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
                return Err(ParseError::Invalid(format!("{}: wasm_dashboard and dashboard are mutually exclusive, pick one", manifest.extension.id)));
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
            name = "Cert-Manager"

            [[extension.kind]]
            group = "cert-manager.io"
            kind = "Certificate"
            category = "Cert-Manager"

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
