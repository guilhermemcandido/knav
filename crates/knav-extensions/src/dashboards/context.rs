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
}

impl<'a> DashboardContext<'a> {
    /// Built once per frame. Dashboards only ever get `&mut Self`, never a way to build one.
    pub fn new(catalog: &'a mut Catalog, client: &'a Client, nodes: &'a [Arc<Node>], node_rows: &'a [NodeRow], events: &'a [EventEntry]) -> Self {
        Self { catalog, client, nodes, node_rows, events }
    }

    /// One CRD kind's manifests, empty if it isn't installed.
    pub fn fetch(&mut self, group: &str, kind: &str) -> Vec<serde_yaml::Value> {
        self.catalog.resolve_crd(group, kind, self.client).map(|w| w.manifests(None)).unwrap_or_default()
    }

    /// Like `fetch`, when only the count is needed.
    pub fn count(&mut self, group: &str, kind: &str) -> usize {
        self.catalog.resolve_crd(group, kind, self.client).map(|w| w.count()).unwrap_or(0)
    }
}
