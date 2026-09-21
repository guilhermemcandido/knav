//! What an object refers to: volumes, environment sources, service accounts and more.

use super::*;

/// What an object refers to: (kind, namespace, name, why).
pub(super) type Use = (String, Option<String>, String, &'static str);

pub(super) fn uses(o: &Obj) -> Vec<Use> {
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
