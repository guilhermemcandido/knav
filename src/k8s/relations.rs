//! How one object relates to the others around it: what owns it and what it
//! owns, what it uses (ConfigMaps, Secrets, volumes, ...), what uses it, and
//! what exposes it. It works on plain manifests, so it needs no cluster calls.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_yaml::Value;

/// Drops what relations never need and secrets should not sit around in memory for:
/// the payload of ConfigMaps and Secrets.
pub fn slim(mut manifest: Value) -> Value {
    if let Some(map) = manifest.as_mapping_mut() {
        for key in ["data", "binaryData", "stringData"] {
            map.remove(key);
        }
    }
    manifest
}

/// Groups show at most this many entries, then "+N more".
const CAP: usize = 12;

/// One related object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    /// Why it is related, or how many pods stand behind it.
    pub detail: String,
    /// Indentation for chains (owner of an owner).
    pub depth: usize,
    /// Can be opened (the "+N more" line can't).
    pub openable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}

/// An object seen through its manifest.
struct Obj<'a> {
    kind: &'a str,
    namespace: Option<&'a str>,
    name: &'a str,
    uid: &'a str,
    manifest: &'a Value,
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(value, |v, key| v.get(*key))?.as_str()
}

fn items<'a>(value: &'a Value, path: &[&str]) -> &'a [Value] {
    path.iter().try_fold(value, |v, key| v.get(*key)).and_then(Value::as_sequence).map(Vec::as_slice).unwrap_or(&[])
}

fn obj(manifest: &Value) -> Option<Obj<'_>> {
    Some(Obj {
        kind: text(manifest, &["kind"])?,
        namespace: text(manifest, &["metadata", "namespace"]),
        name: text(manifest, &["metadata", "name"])?,
        uid: text(manifest, &["metadata", "uid"]).unwrap_or(""),
        manifest,
    })
}

/// Kinds that live outside any namespace.
fn cluster_scoped(kind: &str) -> bool {
    matches!(kind, "Node" | "PersistentVolume" | "StorageClass" | "Namespace")
}

/// The pod spec an object runs: its own for a Pod, the template's for workloads.
fn pod_spec<'a>(o: &Obj<'a>) -> Option<&'a Value> {
    let path: &[&str] = match o.kind {
        "Pod" => &["spec"],
        "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet" | "Job" | "ReplicationController" => &["spec", "template", "spec"],
        "CronJob" => &["spec", "jobTemplate", "spec", "template", "spec"],
        _ => return None,
    };
    path.iter().try_fold(o.manifest, |v, key| v.get(*key))
}

/// The labels the object's pods carry.
fn pod_labels(o: &Obj) -> BTreeMap<String, String> {
    let path: &[&str] = match o.kind {
        "Pod" => &["metadata", "labels"],
        "Deployment" | "ReplicaSet" | "StatefulSet" | "DaemonSet" | "Job" | "ReplicationController" => &["spec", "template", "metadata", "labels"],
        "CronJob" => &["spec", "jobTemplate", "spec", "template", "metadata", "labels"],
        _ => return BTreeMap::new(),
    };
    path.iter()
        .try_fold(o.manifest, |v, key| v.get(*key))
        .and_then(Value::as_mapping)
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default()
}

/// What an object refers to: (kind, namespace, name, why).
type Use = (String, Option<String>, String, &'static str);

fn uses(o: &Obj) -> Vec<Use> {
    let ns = o.namespace.map(String::from);
    let mut out: Vec<Use> = Vec::new();
    let mut add = |kind: &str, namespace: Option<String>, name: &str, why: &'static str| {
        if !name.is_empty() {
            out.push((kind.to_string(), if cluster_scoped(kind) { None } else { namespace }, name.to_string(), why));
        }
    };
    if let Some(spec) = pod_spec(o) {
        for volume in items(spec, &["volumes"]) {
            if let Some(name) = text(volume, &["configMap", "name"]) {
                add("ConfigMap", ns.clone(), name, "volume");
            }
            if let Some(name) = text(volume, &["secret", "secretName"]) {
                add("Secret", ns.clone(), name, "volume");
            }
            if let Some(name) = text(volume, &["persistentVolumeClaim", "claimName"]) {
                add("PersistentVolumeClaim", ns.clone(), name, "volume");
            }
            for source in items(volume, &["projected", "sources"]) {
                if let Some(name) = text(source, &["configMap", "name"]) {
                    add("ConfigMap", ns.clone(), name, "volume");
                }
                if let Some(name) = text(source, &["secret", "name"]) {
                    add("Secret", ns.clone(), name, "volume");
                }
            }
        }
        for container in items(spec, &["containers"]).iter().chain(items(spec, &["initContainers"])) {
            for var in items(container, &["env"]) {
                if let Some(name) = text(var, &["valueFrom", "configMapKeyRef", "name"]) {
                    add("ConfigMap", ns.clone(), name, "env");
                }
                if let Some(name) = text(var, &["valueFrom", "secretKeyRef", "name"]) {
                    add("Secret", ns.clone(), name, "env");
                }
            }
            for source in items(container, &["envFrom"]) {
                if let Some(name) = text(source, &["configMapRef", "name"]) {
                    add("ConfigMap", ns.clone(), name, "env");
                }
                if let Some(name) = text(source, &["secretRef", "name"]) {
                    add("Secret", ns.clone(), name, "env");
                }
            }
        }
        for secret in items(spec, &["imagePullSecrets"]) {
            if let Some(name) = text(secret, &["name"]) {
                add("Secret", ns.clone(), name, "image pull");
            }
        }
        if let Some(name) = text(spec, &["serviceAccountName"]) {
            add("ServiceAccount", ns.clone(), name, "service account");
        }
        if o.kind == "Pod"
            && let Some(node) = text(spec, &["nodeName"])
        {
            add("Node", None, node, "runs on");
        }
    }
    match o.kind {
        "Ingress" => {
            let mut backends: Vec<&str> = items(o.manifest, &["spec", "rules"]).iter().flat_map(|r| items(r, &["http", "paths"])).filter_map(|p| text(p, &["backend", "service", "name"])).collect();
            backends.extend(text(o.manifest, &["spec", "defaultBackend", "service", "name"]));
            for name in backends {
                add("Service", ns.clone(), name, "backend");
            }
            for tls in items(o.manifest, &["spec", "tls"]) {
                if let Some(name) = text(tls, &["secretName"]) {
                    add("Secret", ns.clone(), name, "tls");
                }
            }
        }
        "HorizontalPodAutoscaler" => {
            if let (Some(kind), Some(name)) = (text(o.manifest, &["spec", "scaleTargetRef", "kind"]), text(o.manifest, &["spec", "scaleTargetRef", "name"])) {
                add(kind, ns.clone(), name, "scales");
            }
        }
        "PersistentVolumeClaim" => {
            if let Some(name) = text(o.manifest, &["spec", "volumeName"]) {
                add("PersistentVolume", None, name, "bound to");
            }
            if let Some(name) = text(o.manifest, &["spec", "storageClassName"]) {
                add("StorageClass", None, name, "class");
            }
        }
        "PersistentVolume" => {
            if let (Some(name), Some(claim_ns)) = (text(o.manifest, &["spec", "claimRef", "name"]), text(o.manifest, &["spec", "claimRef", "namespace"])) {
                add("PersistentVolumeClaim", Some(claim_ns.to_string()), name, "claimed by");
            }
            if let Some(name) = text(o.manifest, &["spec", "storageClassName"]) {
                add("StorageClass", None, name, "class");
            }
        }
        _ => {}
    }
    out
}

/// The owner references of an object: (kind, name, uid, controller).
fn owners<'a>(o: &Obj<'a>) -> Vec<(&'a str, &'a str, &'a str, bool)> {
    items(o.manifest, &["metadata", "ownerReferences"])
        .iter()
        .filter_map(|r| Some((text(r, &["kind"])?, text(r, &["name"])?, text(r, &["uid"]).unwrap_or(""), r.get("controller").and_then(Value::as_bool).unwrap_or(false))))
        .collect()
}

fn entry(kind: &str, namespace: Option<&str>, name: &str, detail: impl Into<String>, depth: usize) -> Entry {
    Entry { kind: kind.to_string(), namespace: namespace.map(String::from), name: name.to_string(), detail: detail.into(), depth, openable: true }
}

/// Keeps a group to `CAP` entries and says how many were left out.
fn capped(mut entries: Vec<Entry>) -> Vec<Entry> {
    if entries.len() > CAP {
        let more = entries.len() - CAP;
        entries.truncate(CAP);
        entries.push(Entry { kind: String::new(), namespace: None, name: format!("+{more} more"), detail: String::new(), depth: 0, openable: false });
    }
    entries
}

struct Index<'a> {
    all: Vec<Obj<'a>>,
    by_uid: HashMap<&'a str, usize>,
}

impl<'a> Index<'a> {
    fn new(manifests: &'a [Value]) -> Self {
        let all: Vec<Obj> = manifests.iter().filter_map(obj).collect();
        let by_uid = all.iter().enumerate().filter(|(_, o)| !o.uid.is_empty()).map(|(i, o)| (o.uid, i)).collect();
        Index { all, by_uid }
    }

    fn find(&self, kind: &str, namespace: Option<&str>, name: &str) -> Option<&Obj<'a>> {
        self.all.iter().find(|o| o.kind == kind && o.name == name && (cluster_scoped(kind) || o.namespace == namespace))
    }

    /// The object at the top of an object's owner chain (itself if it has no owner).
    fn top<'s>(&'s self, start: &'s Obj<'a>) -> (&'s str, Option<&'s str>, &'s str) {
        let mut current: (&str, Option<&str>, &str) = (start.kind, start.namespace, start.name);
        let mut object = Some(start);
        for _ in 0..8 {
            let Some(o) = object else { break };
            let Some((kind, name, uid, _)) = owners(o).into_iter().min_by_key(|(_, _, _, controller)| !*controller) else { break };
            current = (kind, o.namespace, name);
            object = self.by_uid.get(uid).map(|i| &self.all[*i]).or_else(|| self.find(kind, o.namespace, name));
        }
        current
    }
}

/// Who runs on behalf of whom: the pods among `objects`, grouped by the object
/// at the top of their owner chain ("Deployment web, 3 pods").
fn by_top_owner(index: &Index, objects: &[&Obj], reason: &str) -> Vec<Entry> {
    let mut groups: BTreeMap<(String, Option<String>, String), (usize, bool)> = BTreeMap::new();
    for o in objects {
        let top = index.top(o);
        let slot = groups.entry((top.0.to_string(), top.1.map(String::from), top.2.to_string())).or_default();
        if o.kind == "Pod" {
            slot.0 += 1;
        }
        slot.1 = true;
    }
    groups
        .into_iter()
        .map(|((kind, namespace, name), (pods, _))| {
            let detail = match pods {
                0 => reason.to_string(),
                1 => "1 pod".to_string(),
                n => format!("{n} pods"),
            };
            Entry { kind, namespace, name, detail, depth: 0, openable: true }
        })
        .collect()
}

/// The groups of relations for `target`, looking through `manifests` (the
/// objects around it; the target itself may be among them).
pub fn relations(target: &Value, manifests: &[Value]) -> Vec<Group> {
    let Some(t) = obj(target) else { return Vec::new() };
    let index = Index::new(manifests);
    let mut groups: Vec<Group> = Vec::new();
    let mut push = |title: &'static str, entries: Vec<Entry>| {
        if !entries.is_empty() {
            groups.push(Group { title, entries: capped(entries) });
        }
    };

    // Owned by: the chain upwards.
    let mut chain = Vec::new();
    let mut current = owners(&t).into_iter().min_by_key(|(_, _, _, controller)| !*controller);
    let mut namespace = t.namespace;
    for depth in 0..8 {
        let Some((kind, name, uid, _)) = current else { break };
        chain.push(entry(kind, namespace, name, if depth == 0 { "controller" } else { "" }, depth));
        let found = index.by_uid.get(uid).map(|i| &index.all[*i]).or_else(|| index.find(kind, namespace, name));
        namespace = found.and_then(|o| o.namespace).or(namespace);
        current = found.and_then(|o| owners(o).into_iter().min_by_key(|(_, _, _, controller)| !*controller));
    }
    push("Owned by", chain);

    // Owns: direct children, with what stands behind each.
    if !t.uid.is_empty() {
        let mut children: Vec<Entry> = index
            .all
            .iter()
            .filter(|o| owners(o).iter().any(|(_, _, uid, _)| *uid == t.uid))
            .map(|child| {
                let below = index.all.iter().filter(|o| !child.uid.is_empty() && owners(o).iter().any(|(_, _, uid, _)| *uid == child.uid)).count();
                let detail = if below > 0 { format!("{below} pod{}", if below == 1 { "" } else { "s" }) } else { String::new() };
                entry(child.kind, child.namespace, child.name, detail, 0)
            })
            .collect();
        children.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));
        push("Owns", children);
    }

    // Uses: what it refers to, each once with every reason.
    let mut used: BTreeMap<(String, Option<String>, String), BTreeSet<&'static str>> = BTreeMap::new();
    for (kind, namespace, name, why) in uses(&t) {
        used.entry((kind, namespace, name)).or_default().insert(why);
    }
    let uses_entries: Vec<Entry> = used.into_iter().map(|((kind, namespace, name), why)| Entry { kind, namespace, name, detail: why.into_iter().collect::<Vec<_>>().join(", "), depth: 0, openable: true }).collect();
    push("Uses", uses_entries);

    // Used by: who refers to it, folded up to the top of their owner chain.
    let users: Vec<&Obj> = index
        .all
        .iter()
        .filter(|o| !(o.kind == t.kind && o.name == t.name && o.namespace == t.namespace))
        .filter(|o| uses(o).iter().any(|(kind, namespace, name, _)| kind == t.kind && name == t.name && (cluster_scoped(t.kind) || namespace.as_deref() == t.namespace)))
        .collect();
    let reason_for = |o: &Obj| uses(o).into_iter().find(|(kind, _, name, _)| kind == t.kind && name == t.name).map(|u| u.3).unwrap_or("");
    let mut used_by = by_top_owner(&index, &users, users.first().map(|o| reason_for(o)).unwrap_or(""));
    // The target's own owners are not "users" of it.
    used_by.retain(|e| !(e.kind == t.kind && e.name == t.name));
    push(if t.kind == "Node" { "Runs" } else { "Used by" }, used_by);

    // Selects (a Service's pods) and exposed by (a workload's Services and their Ingresses).
    if t.kind == "Service" {
        let selector: BTreeMap<String, String> = target.get("spec").and_then(|s| s.get("selector")).and_then(Value::as_mapping).map(|m| m.iter().filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
        if !selector.is_empty() {
            let pods: Vec<&Obj> = index
                .all
                .iter()
                .filter(|o| o.kind == "Pod" && o.namespace == t.namespace)
                .filter(|o| {
                    let labels = pod_labels(o);
                    selector.iter().all(|(k, v)| labels.get(k) == Some(v))
                })
                .collect();
            push("Selects", by_top_owner(&index, &pods, "selected"));
        }
    } else {
        let labels = pod_labels(&t);
        if !labels.is_empty() {
            let mut exposed = Vec::new();
            for service in index.all.iter().filter(|o| o.kind == "Service" && o.namespace == t.namespace) {
                let selector: Vec<(&str, &str)> = service.manifest.get("spec").and_then(|s| s.get("selector")).and_then(Value::as_mapping).map(|m| m.iter().filter_map(|(k, v)| Some((k.as_str()?, v.as_str()?))).collect()).unwrap_or_default();
                if selector.is_empty() || !selector.iter().all(|(k, v)| labels.get(*k).map(String::as_str) == Some(*v)) {
                    continue;
                }
                exposed.push(entry("Service", service.namespace, service.name, "selects it", 0));
                for ingress in index.all.iter().filter(|o| o.kind == "Ingress" && o.namespace == service.namespace) {
                    if uses(ingress).iter().any(|(kind, _, name, _)| kind == "Service" && name == service.name) {
                        exposed.push(entry("Ingress", ingress.namespace, ingress.name, "routes to the service", 1));
                    }
                }
            }
            push("Exposed by", exposed);
        }
    }
    groups
}

/// One box of the diagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode {
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub detail: String,
    /// Columns left (negative) and right (positive) of the object in the middle.
    pub layer: i32,
    pub openable: bool,
}

/// The relations as boxes and arrows (from, to). Node 0 is the object itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<(usize, usize)>,
}

/// Lays `groups` out as a flow from left to right: callers and owners on the left,
/// the object in the middle, what it owns and uses on the right.
pub fn graph(target: &Value, groups: &[Group]) -> Graph {
    let mut g = Graph::default();
    let Some(t) = obj(target) else { return g };
    g.nodes.push(GraphNode { kind: t.kind.to_string(), namespace: t.namespace.map(String::from), name: t.name.to_string(), detail: String::new(), layer: 0, openable: true });
    fn node(g: &mut Graph, e: &Entry, layer: i32) -> usize {
        if e.openable
            && let Some(at) = g.nodes.iter().position(|n| n.openable && n.kind == e.kind && n.name == e.name && n.namespace == e.namespace)
        {
            return at;
        }
        g.nodes.push(GraphNode { kind: e.kind.clone(), namespace: e.namespace.clone(), name: e.name.clone(), detail: e.detail.clone(), layer, openable: e.openable });
        g.nodes.len() - 1
    }
    for group in groups {
        match group.title {
            "Owned by" => {
                let mut previous = 0;
                for e in &group.entries {
                    let at = node(&mut g, e, -(e.depth as i32) - 1);
                    g.edges.push((at, previous));
                    previous = at;
                }
            }
            "Used by" => {
                for e in &group.entries {
                    let at = node(&mut g, e, -1);
                    g.edges.push((at, 0));
                }
            }
            "Exposed by" => {
                let mut service = 0;
                for e in &group.entries {
                    if e.depth == 0 {
                        service = node(&mut g, e, -1);
                        g.edges.push((service, 0));
                    } else {
                        let ingress = node(&mut g, e, -2);
                        g.edges.push((ingress, service));
                    }
                }
            }
            _ => {
                for e in &group.entries {
                    let at = node(&mut g, e, 1);
                    g.edges.push((0, at));
                }
            }
        }
    }
    g
}

/// The manifest of `kind`/`name` among `all`.
pub fn find_manifest(all: &[Value], kind: &str, namespace: Option<&str>, name: &str) -> Option<Value> {
    Index::new(all).find(kind, namespace, name).map(|o| o.manifest.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn v(value: serde_json::Value) -> Value {
        serde_yaml::to_value(value).unwrap()
    }

    fn deployment() -> Value {
        v(json!({"kind": "Deployment", "metadata": {"name": "web", "namespace": "shop", "uid": "d1"},
            "spec": {"template": {"metadata": {"labels": {"app": "web"}}, "spec": {"serviceAccountName": "web-sa",
                "containers": [{"name": "c", "envFrom": [{"configMapRef": {"name": "app-config"}}], "env": [{"name": "P", "valueFrom": {"secretKeyRef": {"name": "db-creds", "key": "p"}}}]}],
                "volumes": [{"name": "data", "persistentVolumeClaim": {"claimName": "data"}}]}}}}))
    }

    fn replica_set() -> Value {
        v(json!({"kind": "ReplicaSet", "metadata": {"name": "web-5d9d", "namespace": "shop", "uid": "r1", "ownerReferences": [{"kind": "Deployment", "name": "web", "uid": "d1", "controller": true}]},
            "spec": {"template": {"metadata": {"labels": {"app": "web"}}, "spec": {"containers": []}}}}))
    }

    fn pod(name: &str, uid: &str) -> Value {
        v(json!({"kind": "Pod", "metadata": {"name": name, "namespace": "shop", "uid": uid, "labels": {"app": "web"}, "ownerReferences": [{"kind": "ReplicaSet", "name": "web-5d9d", "uid": "r1", "controller": true}]},
            "spec": {"nodeName": "node-1", "serviceAccountName": "web-sa", "containers": [{"name": "c", "envFrom": [{"configMapRef": {"name": "app-config"}}]}]}}))
    }

    fn service() -> Value {
        v(json!({"kind": "Service", "metadata": {"name": "web", "namespace": "shop", "uid": "s1"}, "spec": {"selector": {"app": "web"}}}))
    }

    fn ingress() -> Value {
        v(json!({"kind": "Ingress", "metadata": {"name": "web-ing", "namespace": "shop", "uid": "i1"},
            "spec": {"rules": [{"http": {"paths": [{"backend": {"service": {"name": "web"}}}]}}], "tls": [{"secretName": "tls-cert"}]}}))
    }

    fn config_map() -> Value {
        v(json!({"kind": "ConfigMap", "metadata": {"name": "app-config", "namespace": "shop", "uid": "c1"}}))
    }

    fn world() -> Vec<Value> {
        vec![deployment(), replica_set(), pod("web-x", "p1"), pod("web-y", "p2"), service(), ingress(), config_map()]
    }

    fn group<'a>(groups: &'a [Group], title: &str) -> &'a Group {
        groups.iter().find(|g| g.title == title).unwrap_or_else(|| panic!("no {title} in {:?}", groups.iter().map(|g| g.title).collect::<Vec<_>>()))
    }

    fn names(group: &Group) -> Vec<String> {
        group.entries.iter().map(|e| format!("{} {}", e.kind, e.name)).collect()
    }

    #[test]
    fn a_pod_is_owned_up_the_chain() {
        let groups = relations(&pod("web-x", "p1"), &world());
        let owned = group(&groups, "Owned by");
        assert_eq!(names(owned), ["ReplicaSet web-5d9d", "Deployment web"]);
        assert_eq!(owned.entries[1].depth, 1);
    }

    #[test]
    fn a_deployment_owns_its_replica_sets_with_their_pod_counts() {
        let groups = relations(&deployment(), &world());
        let owns = group(&groups, "Owns");
        assert_eq!(names(owns), ["ReplicaSet web-5d9d"]);
        assert_eq!(owns.entries[0].detail, "2 pods");
    }

    #[test]
    fn a_pod_uses_config_secrets_volumes_account_and_node() {
        let groups = relations(&pod("web-x", "p1"), &world());
        let uses = group(&groups, "Uses");
        assert_eq!(names(uses), ["ConfigMap app-config", "Node node-1", "ServiceAccount web-sa"]);
        assert_eq!(uses.entries[1].detail, "runs on");
    }

    #[test]
    fn a_deployment_uses_what_its_template_uses() {
        let groups = relations(&deployment(), &world());
        let uses = group(&groups, "Uses");
        assert_eq!(names(uses), ["ConfigMap app-config", "PersistentVolumeClaim data", "Secret db-creds", "ServiceAccount web-sa"]);
    }

    #[test]
    fn a_config_map_is_used_by_the_top_of_its_users_chains() {
        let groups = relations(&config_map(), &world());
        let used = group(&groups, "Used by");
        assert_eq!(names(used), ["Deployment web"]);
        assert_eq!(used.entries[0].detail, "2 pods");
    }

    #[test]
    fn a_workload_is_exposed_by_services_and_their_ingresses() {
        let groups = relations(&pod("web-x", "p1"), &world());
        let exposed = group(&groups, "Exposed by");
        assert_eq!(names(exposed), ["Service web", "Ingress web-ing"]);
        assert_eq!(exposed.entries[1].depth, 1);
    }

    #[test]
    fn a_service_selects_pods_folded_by_owner_and_is_used_by_ingresses() {
        let groups = relations(&service(), &world());
        assert_eq!(names(group(&groups, "Selects")), ["Deployment web"]);
        assert_eq!(names(group(&groups, "Used by")), ["Ingress web-ing"]);
    }

    #[test]
    fn a_node_runs_the_owners_of_its_pods() {
        let node = v(json!({"kind": "Node", "metadata": {"name": "node-1", "uid": "n1"}}));
        let groups = relations(&node, &world());
        let runs = group(&groups, "Runs");
        assert_eq!(names(runs), ["Deployment web"]);
        assert_eq!(runs.entries[0].detail, "2 pods");
    }

    #[test]
    fn a_long_group_is_capped() {
        let mut world = world();
        for i in 0..20 {
            world.push(v(json!({"kind": "Pod", "metadata": {"name": format!("lone-{i}"), "namespace": "shop", "uid": format!("l{i}")}, "spec": {"containers": [{"name": "c", "envFrom": [{"configMapRef": {"name": "app-config"}}]}]}})));
        }
        let groups = relations(&config_map(), &world);
        let used = group(&groups, "Used by");
        assert_eq!(used.entries.len(), CAP + 1);
        assert!(!used.entries.last().unwrap().openable);
    }

    #[test]
    fn the_graph_flows_from_callers_through_the_object_to_what_it_uses() {
        let target = pod("web-x", "p1");
        let g = graph(&target, &relations(&target, &world()));
        let layer = |kind: &str| g.nodes.iter().find(|n| n.kind == kind).map(|n| n.layer);
        assert_eq!(g.nodes[0].layer, 0);
        assert_eq!(layer("ReplicaSet"), Some(-1));
        assert_eq!(layer("Deployment"), Some(-2));
        assert_eq!(layer("Ingress"), Some(-2));
        assert_eq!(layer("Service"), Some(-1));
        assert_eq!(layer("ConfigMap"), Some(1));
        let at = |kind: &str| g.nodes.iter().position(|n| n.kind == kind).unwrap();
        assert!(g.edges.contains(&(at("Deployment"), at("ReplicaSet"))), "owner points at what it owns");
        assert!(g.edges.contains(&(at("ReplicaSet"), 0)));
        assert!(g.edges.contains(&(at("Ingress"), at("Service"))));
        assert!(g.edges.contains(&(0, at("ConfigMap"))));
    }

    #[test]
    fn a_manifest_can_be_found_by_kind_and_name() {
        assert!(find_manifest(&world(), "Service", Some("shop"), "web").is_some());
        assert!(find_manifest(&world(), "Service", Some("shop"), "nope").is_none());
    }

    #[test]
    fn an_object_with_no_relations_has_no_groups() {
        assert!(relations(&v(json!({"kind": "ConfigMap", "metadata": {"name": "lonely", "namespace": "x", "uid": "z"}})), &[]).is_empty());
    }
}
