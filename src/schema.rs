use serde_json::Value;
pub fn is_write(name: &str) -> bool {
    name.starts_with("create_") || name.starts_with("update_") || name.starts_with("delete_")
}
pub fn payload(result: &Value) -> Value {
    if let Some(value) = result.get("structuredContent") {
        return value.clone();
    }
    let texts: Vec<_> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect();
    if texts.len() == 1
        && let Ok(value) = serde_json::from_str(texts[0])
    {
        return value;
    }
    if texts.is_empty() {
        result.clone()
    } else {
        Value::String(texts.join("\n\n"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn normalizes_structured_and_text_payloads() {
        assert_eq!(
            payload(&json!({"structuredContent":{"courses":[]}})),
            json!({"courses":[]})
        );
        assert_eq!(
            payload(&json!({"content":[{"text":"{\"courses\":[]}"}]})),
            json!({"courses":[]})
        );
    }
}
