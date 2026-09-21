use super::*;
use serde_json::json;

fn v(value: serde_json::Value) -> Value {
    serde_yaml::to_value(value).unwrap()
}

fn pod() -> Value {
    v(json!({"apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "web-x", "namespace": "shop", "creationTimestamp": "2020-01-01T00:00:00Z", "labels": {"app": "web", "tier": "front"},
            "ownerReferences": [{"kind": "ReplicaSet", "name": "web-5d9d"}]},
        "spec": {"nodeName": "node-1", "serviceAccountName": "web-sa", "containers": [{"name": "nginx", "image": "nginx:1.25", "ports": [{"containerPort": 80}],
            "resources": {"requests": {"cpu": "100m", "memory": "64Mi"}, "limits": {"cpu": "200m"}}, "env": [{"name": "A", "value": "1"}]}]},
        "status": {"phase": "Running", "podIP": "10.0.0.5", "containerStatuses": [{"name": "nginx", "ready": true, "restartCount": 2, "state": {"running": {"startedAt": "2020-01-01T00:00:00Z"}}}],
            "conditions": [{"type": "Ready", "status": "True"}, {"type": "PodScheduled", "status": "True"}]}}))
}

fn section<'a>(sections: &'a [Section], title: &str) -> &'a Section {
    sections.iter().find(|s| s.title == title).unwrap_or_else(|| panic!("no {title}: {:?}", sections.iter().map(|s| &s.title).collect::<Vec<_>>()))
}

fn label_value(section: &Section, label: &str) -> String {
    section.lines.iter().find_map(|l| match l {
        Line::Field(name, chunks) if name == label => Some(chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" ")),
        _ => None,
    }).unwrap_or_else(|| panic!("no field {label}"))
}

#[test]
fn every_object_gets_its_name_namespace_labels_and_owner() {
    let sections = details(&pod(), &[], false);
    let props = section(&sections, "Properties");
    assert_eq!(label_value(props, "Name"), "web-x");
    assert_eq!(label_value(props, "Namespace"), "shop");
    assert_eq!(label_value(props, "Labels"), "app=web tier=front");
    assert_eq!(label_value(props, "Controlled by"), "ReplicaSet web-5d9d");
}

#[test]
fn a_pod_shows_status_node_and_each_container() {
    let sections = details(&pod(), &[], false);
    assert_eq!(label_value(section(&sections, "Status"), "Node"), "node-1");
    let containers = section(&sections, "Containers");
    let text: String = containers.lines.iter().map(|l| format!("{l:?}")).collect();
    assert!(text.contains("nginx:1.25") && text.contains("80/TCP") && text.contains("cpu 100m") && text.contains("2 restarts") && text.contains("ready"), "{text}");
}

#[test]
fn conditions_are_toned_and_true_pressure_is_bad() {
    let node = v(json!({"kind": "Node", "metadata": {"name": "n"}, "status": {"conditions": [{"type": "Ready", "status": "True"}, {"type": "MemoryPressure", "status": "True"}]}}));
    let sections = details(&node, &[], false);
    let conditions = section(&sections, "Conditions");
    let styles: Vec<Style> = conditions.lines.iter().filter_map(|l| if let Line::Item(c) = l { Some(c[1].style) } else { None }).collect();
    assert_eq!(styles, [Style::Good, Style::Bad]);
}

#[test]
fn secrets_list_key_names_and_never_values() {
    let secret = v(json!({"kind": "Secret", "metadata": {"name": "s"}, "type": "Opaque", "data": {"password": "c2VjcmV0", "user": "YWRtaW4="}}));
    let sections = details(&secret, &[], false);
    let all = format!("{sections:?}");
    assert!(all.contains("password") && all.contains("user"));
    assert!(!all.contains("c2VjcmV0") && !all.contains("YWRtaW4="));
}

#[test]
fn config_maps_show_their_values_and_pods_show_their_environment() {
    let map = v(json!({"kind": "ConfigMap", "metadata": {"name": "c"}, "data": {"Corefile": ".:53 {\n  errors\n}", "mode": "fast"}}));
    let text = format!("{:?}", details(&map, &[], false));
    assert!(text.contains("errors") && text.contains("fast") && text.contains("Corefile"), "{text}");
    let pod = v(json!({"kind": "Pod", "metadata": {"name": "p"}, "spec": {"containers": [{"name": "c", "image": "i",
        "env": [{"name": "A", "value": "1"}, {"name": "B", "valueFrom": {"secretKeyRef": {"name": "db", "key": "pw"}}}], "envFrom": [{"configMapRef": {"name": "cfg"}}],
        "volumeMounts": [{"name": "data", "mountPath": "/data", "readOnly": true}]}]}}));
    let text = format!("{:?}", details(&pod, &[], false));
    assert!(text.contains("\"A\"") && text.contains("secret db / pw") && text.contains("every key of configMap cfg") && text.contains("/data") && text.contains("read only"), "{text}");
}

#[test]
fn a_service_lists_ports_and_selector() {
    let service = v(json!({"kind": "Service", "metadata": {"name": "web", "namespace": "shop"}, "spec": {"type": "ClusterIP", "clusterIP": "10.0.0.1", "ports": [{"port": 80, "targetPort": 8080, "protocol": "TCP"}], "selector": {"app": "web"}}}));
    let sections = details(&service, &[], false);
    let s = section(&sections, "Service");
    assert_eq!(label_value(s, "Port"), "80 → 8080/TCP");
    assert_eq!(label_value(s, "Selector"), "app=web");
}

#[test]
fn events_for_the_object_are_included() {
    let events = vec![EventEntry { message: "Back-off restarting".into(), reason: "BackOff".into(), object: "web-x".into(), namespace: "shop".into(), kind: "Pod".into(), age: "3m".into(), age_secs: 180, severity: crate::k8s::EventSeverity::Warning }];
    let sections = details(&pod(), &events, false);
    assert_eq!(section(&sections, "Events").lines.len(), 1);
    assert!(details(&pod(), &[], false).iter().all(|s| s.title != "Events"));
}

#[test]
fn an_unknown_kind_still_gets_properties_and_conditions() {
    let widget = v(json!({"kind": "Widget", "apiVersion": "x/v1", "metadata": {"name": "w"}, "status": {"conditions": [{"type": "Ready", "status": "False", "reason": "Broken"}]}}));
    let sections = details(&widget, &[], false);
    assert_eq!(sections.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Properties", "Conditions"]);
}

#[test]
fn a_role_lists_its_rules_with_verbs_and_flags_broad_access() {
    let role = v(json!({"kind": "ClusterRole", "metadata": {"name": "r"}, "rules": [
        {"apiGroups": [""], "resources": ["pods", "pods/exec"], "verbs": ["get", "list"]},
        {"apiGroups": ["apps"], "resources": ["deployments"], "resourceNames": ["web"], "verbs": ["*"]}]}));
    let sections = details(&role, &[], false);
    assert_eq!(label_value(section(&sections, "Role"), "Rules"), "2");
    assert!(label_value(section(&sections, "Role"), "Broad access").contains("1 rule"));
    let text = format!("{:?}", section(&sections, "Rules"));
    assert!(text.contains("core/pods/exec") && text.contains("apps/deployments") && text.contains("\"get\"") && text.contains("web"), "{text}");
}

#[test]
fn a_binding_shows_the_role_and_each_subject() {
    let binding = v(json!({"kind": "RoleBinding", "metadata": {"name": "b", "namespace": "shop"}, "roleRef": {"kind": "ClusterRole", "name": "cluster-admin", "apiGroup": "rbac.authorization.k8s.io"},
        "subjects": [{"kind": "ServiceAccount", "name": "web"}, {"kind": "User", "name": "ana"}]}));
    let sections = details(&binding, &[], false);
    assert_eq!(label_value(section(&sections, "Binding"), "Grants"), "ClusterRole cluster-admin");
    let text = format!("{:?}", section(&sections, "Subjects"));
    assert!(text.contains("web") && text.contains("in shop") && text.contains("ana"), "{text}");
}

#[test]
fn last_applied_annotation_is_hidden_so_secret_data_cannot_leak() {
    let secret = v(json!({"kind": "Secret", "metadata": {"name": "s", "annotations": {"kubectl.kubernetes.io/last-applied-configuration": "{\"data\":{\"pw\":\"aHVudGVyMg==\"}}", "team": "x"}}, "data": {"pw": "aHVudGVyMg=="}}));
    let text = format!("{:?}", details(&secret, &[], false));
    assert!(!text.contains("aHVudGVyMg==") && text.contains("team=x"), "{text}");
}

#[test]
fn events_of_a_same_named_object_in_another_namespace_are_left_out() {
    let mut other = EventEntry { message: "m".into(), reason: "r".into(), object: "web-x".into(), namespace: "elsewhere".into(), kind: "Pod".into(), age: "1m".into(), age_secs: 60, severity: crate::k8s::EventSeverity::Normal };
    assert!(details(&pod(), std::slice::from_ref(&other), false).iter().all(|s| s.title != "Events"));
    other.namespace = "shop".into();
    assert!(details(&pod(), &[other], false).iter().any(|s| s.title == "Events"));
}

#[test]
fn quotas_show_usage_against_the_limit() {
    let quota = v(json!({"kind": "ResourceQuota", "metadata": {"name": "q"}, "status": {"hard": {"pods": "10", "cpu": "2"}, "used": {"pods": "9", "cpu": "500m"}}}));
    let text = format!("{:?}", details(&quota, &[], false));
    assert!(text.contains("9 / 10") && text.contains("90%") && text.contains("25%"), "{text}");
}

#[test]
fn secret_values_appear_only_when_revealed() {
    let secret = v(json!({"kind": "Secret", "metadata": {"name": "s"}, "data": {"password": "c2VjcmV0", "blob": "/w=="}}));
    let hidden = format!("{:?}", details(&secret, &[], false));
    assert!(!hidden.contains("secret\"") && hidden.contains("x shows them"), "{hidden}");
    let shown = format!("{:?}", details(&secret, &[], true));
    assert!(shown.contains("secret") && shown.contains("x hides them"), "{shown}");
    assert!(shown.contains("blob") && !shown.contains("/w=="), "binary stays a size");
}
