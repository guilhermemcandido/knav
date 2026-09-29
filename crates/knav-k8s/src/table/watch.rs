//! Keeps a type's Table rows current from a watch that asks for the Table format, so a big
//! list is read once instead of again every few seconds.

use futures::AsyncBufReadExt;
use futures::StreamExt;
use kube::{Client, api::ApiResource};
use serde_json::Value;
use std::cmp::Ordering;
use std::sync::Mutex;

use super::{TableData, TableRow, list_path, parse_table, percent_encode};

/// The server closes a watch after about this long; it is simply opened again.
const WATCH_SECONDS: u32 = 290;

/// The order the server lists in: `namespace/name`, or just the name when cluster-scoped.
pub(super) fn order(a: &TableRow, b: &TableRow) -> Ordering {
    fn key(row: &TableRow) -> impl Iterator<Item = u8> + '_ {
        let namespace = if row.namespace == "-" { "" } else { row.namespace.as_str() };
        namespace.bytes().chain((!namespace.is_empty()).then_some(b'/')).chain(row.name.bytes())
    }
    key(a).cmp(key(b))
}

/// Puts rows in that order if they are not (the lookups below rely on it).
pub(super) fn ensure_ordered(rows: &mut [TableRow]) {
    if !rows.is_sorted_by(|a, b| order(a, b) != Ordering::Greater) {
        rows.sort_by(order);
    }
}

/// Applies one watch event's rows: `DELETED` removes them, anything else adds or replaces.
fn apply(data: &mut TableData, event: &str, rows: Vec<TableRow>) {
    for row in rows {
        match (event, data.rows.binary_search_by(|held| order(held, &row))) {
            ("DELETED", Ok(at)) => {
                data.rows.remove(at);
            }
            ("DELETED", Err(_)) => {}
            (_, Ok(at)) => data.rows[at] = row,
            (_, Err(at)) => data.rows.insert(at, row),
        }
    }
}

/// Follows changes from `version` on, reopening the watch when the server closes it. Returns
/// once the rows can no longer be trusted (the version is too old, the connection fails), so
/// the caller reads the list again.
pub(super) async fn watch_table(client: &Client, resource: &ApiResource, data: &Mutex<TableData>, namespace: Option<&str>, version: &mut String) -> bool {
    let base = list_path(resource, namespace);
    loop {
        let path = format!("{base}?watch=true&allowWatchBookmarks=true&timeoutSeconds={WATCH_SECONDS}&resourceVersion={}", percent_encode(version));
        let Ok(request) = http::Request::get(path).header(http::header::ACCEPT, "application/json;as=Table;g=meta.k8s.io;v=v1").body(Vec::new()) else { return true };
        let Ok(stream) = client.request_stream(request).await else { return true };
        let opened = std::time::Instant::now();
        let mut lines = stream.lines();
        while let Some(line) = lines.next().await {
            let Ok(line) = line else { return true };
            let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
            let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();
            let Some(object) = event.get("object") else { continue };
            if kind == "ERROR" {
                return true;
            }
            if let Some(next) = object.get("metadata").and_then(|m| m.get("resourceVersion")).and_then(Value::as_str) {
                *version = next.to_string();
            }
            if kind == "BOOKMARK" {
                continue;
            }
            if let Ok((_, rows)) = parse_table(object)
                && let Ok(mut held) = data.lock()
            {
                apply(&mut held, kind, rows);
                super::note_change();
            }
        }
        // A watch that closes at once would be reopened in a tight loop.
        if opened.elapsed() < std::time::Duration::from_secs(1) {
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(namespace: &str, name: &str, cell: &str) -> TableRow {
        TableRow { cells: vec![cell.into()], namespace: namespace.into(), name: name.into(), uid: String::new(), owners: Vec::new(), created: None, labels: String::new() }
    }

    fn names(data: &TableData) -> Vec<String> {
        data.rows.iter().map(|r| format!("{}/{}={}", r.namespace, r.name, r.cells[0])).collect()
    }

    #[test]
    fn events_add_replace_and_remove_rows_in_the_servers_order() {
        let mut data = TableData::default();
        apply(&mut data, "ADDED", vec![row("b", "x", "1"), row("a", "y", "1")]);
        apply(&mut data, "ADDED", vec![row("b", "a", "1")]);
        assert_eq!(names(&data), ["a/y=1", "b/a=1", "b/x=1"]);
        apply(&mut data, "MODIFIED", vec![row("a", "y", "2")]);
        apply(&mut data, "DELETED", vec![row("b", "x", "1"), row("b", "missing", "1")]);
        assert_eq!(names(&data), ["a/y=2", "b/a=1"]);
    }

    #[test]
    fn the_order_is_the_key_order_not_namespace_then_name() {
        // "a-b/x" sorts before "a/x" as keys, though the namespace "a" is the shorter one.
        assert_eq!(order(&row("a-b", "x", ""), &row("a", "x", "")), Ordering::Less);
        let mut rows = vec![row("a", "x", ""), row("a-b", "x", "")];
        ensure_ordered(&mut rows);
        assert_eq!(rows[0].namespace, "a-b");
    }
}
