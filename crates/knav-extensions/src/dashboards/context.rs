//! What a dashboard can read. `catalog` and `client` are private to this module, which the
//! dashboards are siblings of, so they can't reach the client; `fetch` and `count` only
//! watch, and `Catalog` has no way to patch, create or delete.

use std::sync::Arc;

use k8s_openapi::api::core::v1::Node;
use kube::Client;

use knav_k8s::catalog::Catalog;
use knav_k8s::{EventEntry, NodeRow};

pub struct DashboardContext<'a> {
    catalog: &'a mut Catalog,
    client: &'a Client,
    pub nodes: &'a [Arc<Node>],
    pub node_rows: &'a [NodeRow],
    pub events: &'a [EventEntry],
    /// Cells a dashboard has to draw in, so it can lay out cards and bars.
    pub width: usize,
    /// Objects still arriving in the background, over everything fetched so far.
    missing: usize,
}

impl<'a> DashboardContext<'a> {
    /// Built once per frame. Dashboards only ever get `&mut Self`, never a way to build one.
    pub fn new(catalog: &'a mut Catalog, client: &'a Client, nodes: &'a [Arc<Node>], node_rows: &'a [NodeRow], events: &'a [EventEntry], width: usize) -> Self {
        Self { catalog, client, nodes, node_rows, events, width, missing: 0 }
    }

    /// One CRD kind's manifests, empty if it isn't installed.
    /// Never waits on the network: objects not fetched yet come in the background and are
    /// counted in `missing`.
    pub fn fetch(&mut self, group: &str, kind: &str) -> Vec<serde_yaml::Value> {
        let (found, missing) = self.catalog.resolve_crd(group, kind, self.client).map(|w| w.peek_manifests()).unwrap_or_default();
        self.missing += missing;
        found
    }

    /// How many objects the dashboard is still waiting for.
    pub fn missing(&self) -> usize {
        self.missing
    }

    /// Like `fetch`, when only the count is needed.
    pub fn count(&mut self, group: &str, kind: &str) -> usize {
        self.catalog.resolve_crd(group, kind, self.client).map(|w| w.count()).unwrap_or(0)
    }
}
