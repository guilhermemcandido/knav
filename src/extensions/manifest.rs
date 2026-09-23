//! The shape of an extension manifest (`extensions/*.toml`), and nothing else: a
//! manifest can only select from the templates this crate already implements,
//! never supply code of its own. See `Registry` for how these load.
//!
//! Extensions are read-only by design: a manifest can attach a category, icon
//! and detail view to CRD kinds the cluster already has, but has no mechanism
//! to patch, annotate, create or delete anything. `category`/`icon`/`kinds` are
//! wired up (see `Catalog::extension_sections`); `view` is parsed and validated
//! here, and covered by the tests below, but not yet reachable from a keypress
//! — the next round wires the detail-view templates.
#![allow(dead_code)]

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
    pub icon: Option<String>,
    /// Reserved for the detail-view content template; parsed and validated,
    /// not yet rendered.
    #[serde(default)]
    pub view: Option<ViewTemplate>,
}

/// A view template is a fixed choice, not a rendering instruction: the
/// manifest supplies a field path, this crate supplies how it's drawn.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "template", rename_all = "snake_case")]
pub enum ViewTemplate {
    /// Reuses the same conditions renderer built-in kinds already have.
    Timeline { from: String },
    Health { from: String, ok: String },
    KeyValues { fields: Vec<[String; 2]> },
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
    fn a_blank_id_is_rejected() {
        assert!(Manifest::parse("[extension]\nid = \"\"\nname = \"x\"\n").is_err());
    }

    #[test]
    fn malformed_toml_is_a_parse_error_not_a_panic() {
        assert!(Manifest::parse("not valid toml [[[").is_err());
    }
}
