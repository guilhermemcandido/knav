//! Reading a Secret's values.

/// A Secret's manifest with each `data` value decoded from base64, for
/// reading it. A value that isn't UTF-8 text shows its size instead.
pub fn decode_secret(manifest: &serde_yaml::Value) -> serde_yaml::Value {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut decoded = manifest.clone();
    if let Some(data) = decoded.get_mut("data").and_then(|d| d.as_mapping_mut()) {
        for (_, value) in data.iter_mut() {
            let Some(encoded) = value.as_str() else { continue };
            *value = match STANDARD.decode(encoded.trim()) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => serde_yaml::Value::String(text),
                    Err(e) => serde_yaml::Value::String(format!("<binary, {} bytes>", e.as_bytes().len())),
                },
                Err(_) => serde_yaml::Value::String(format!("<not base64: {encoded}>")),
            };
        }
    }
    decoded
}

