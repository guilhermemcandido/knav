

/// Seconds since creation, for sorting by AGE — unknown ages sort last
/// when ascending.
pub(super) fn age_seconds(created: Option<&k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>) -> i64 {
    created.map(|t| (k8s_openapi::jiff::Timestamp::now().as_second() - t.0.as_second()).max(0)).unwrap_or(i64::MAX)
}

/// A short "5m"/"3h"/"2d" style duration, matching kubectl/k9s's AGE
/// column convention (single dominant unit, not a full breakdown).
pub fn humanize_age(created: k8s_openapi::jiff::Timestamp) -> String {
    let secs = (k8s_openapi::jiff::Timestamp::now().as_second() - created.as_second()).max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}
