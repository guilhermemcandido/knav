//! What a `Dashboard` gets to touch, and — just as important — what it
//! can't. `catalog` and `client` are private, in their own module the native
//! dashboards (`karpenter`, `gitops`) and the declarative interpreter are
//! siblings of, not descendants of: Rust gives a private field to a module's
//! descendants too, so keeping them out of the same subtree is what actually
//! blocks `ctx.catalog`/`ctx.client` field access from those modules, not
//! just a naming convention. `fetch`/`count` are the only way back in, and
//! both only ever call `Catalog::resolve_crd`, which only watches — there is
//! no `patch`/`create`/`delete` anywhere in `Catalog`'s API. A `Dashboard`
//! implementation, bundled or `knav ext add`-installed alike, has no way to
//! reach the raw `kube::Client` (and its `patch`/`create`/`delete`) at all.

use std::sync::Arc;

use k8s_openapi::api::core::v1::Node;
use kube::Client;

use crate::Catalog;
use crate::k8s::{EventEntry, NodeRow};

pub struct DashboardContext<'a> {
    catalog: &'a mut Catalog,
    client: &'a Client,
    pub nodes: &'a [Arc<Node>],
    pub node_rows: &'a [NodeRow],
    pub events: &'a [EventEntry],
}

impl<'a> DashboardContext<'a> {
    /// Built once per frame from data `derive` already holds. The only
    /// constructor — a `Dashboard` never builds or clones one itself, so it
    /// only ever sees `&mut Self`, never anything it could construct a wider
    /// one from.
    pub(crate) fn new(catalog: &'a mut Catalog, client: &'a Client, nodes: &'a [Arc<Node>], node_rows: &'a [NodeRow], events: &'a [EventEntry]) -> Self {
        Self { catalog, client, nodes, node_rows, events }
    }

    /// One CRD kind's manifests — empty if it isn't installed. The one door
    /// a dashboard has into the catalog; it never touches CRD watches directly.
    pub fn fetch(&mut self, group: &str, kind: &str) -> Vec<serde_yaml::Value> {
        self.catalog.resolve_crd(group, kind, self.client).map(|w| w.manifests(None)).unwrap_or_default()
    }

    /// Like `fetch`, when only the count is needed (cheaper to read, though
    /// the watch behind it is the same either way).
    pub fn count(&mut self, group: &str, kind: &str) -> usize {
        self.catalog.resolve_crd(group, kind, self.client).map(|w| w.count()).unwrap_or(0)
    }
}
