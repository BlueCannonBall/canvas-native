//! Small in-process result types. No MCP router, schema generator or transport.
use serde::Serialize;
use serde_json::Value;

pub struct Parameters<T>(pub T);
#[derive(Debug)]
pub struct CanvasError {
    pub message: String,
}
impl CanvasError {
    pub fn internal_error(message: impl Into<String>, _details: Option<Value>) -> Self {
        Self {
            message: message.into(),
        }
    }
    pub fn invalid_params(message: impl Into<String>, _details: Option<Value>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
#[derive(Serialize)]
pub struct CanvasResult {
    pub content: Vec<Content>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
}
impl CanvasResult {
    pub fn structured(value: Value) -> Self {
        Self {
            content: vec![],
            structured_content: Some(value),
        }
    }
    pub fn success(content: Vec<Content>) -> Self {
        Self {
            content,
            structured_content: None,
        }
    }
}
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}
impl Content {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self::Image {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }
}
#[derive(Serialize)]
pub struct ResourceContent {
    uri: String,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
}
impl ResourceContent {
    pub fn text(text: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            mime_type: None,
            text: Some(text.into()),
            blob: None,
        }
    }
    pub fn blob(blob: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            mime_type: None,
            text: None,
            blob: Some(blob.into()),
        }
    }
    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }
}
#[derive(Serialize)]
pub struct ResourceResult {
    contents: Vec<ResourceContent>,
}
impl ResourceResult {
    pub fn new(contents: Vec<ResourceContent>) -> Self {
        Self { contents }
    }
}

pub const OPERATION_NAMES: &[&str] = &[
    "assignment_grade",
    "assignment_info",
    "assignment_list",
    "attachment_download",
    "attachment_image",
    "attachment_text",
    "auth",
    "course_grades",
    "course_info",
    "course_list",
    "create_custom_planner_item",
    "create_planner_note",
    "dashboard_items",
    "delete_custom_planner_item",
    "delete_planner_note",
    "download_attachment",
    "file_info",
    "grade_summary",
    "module_list",
    "page_info",
    "planner_note_info",
    "refresh_auth",
    "update_custom_planner_item",
    "update_planner_note",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn structured_and_image_payloads_preserve_ui_contract() {
        let result = serde_json::to_value(CanvasResult::structured(json!({"courses":[]}))).unwrap();
        assert_eq!(result["structuredContent"]["courses"], json!([]));
        let result = serde_json::to_value(CanvasResult::success(vec![Content::image(
            "AQID",
            "image/png",
        )]))
        .unwrap();
        assert_eq!(
            result["content"][0],
            json!({"type":"image","mimeType":"image/png","data":"AQID"})
        );
    }
    #[test]
    fn resource_payload_preserves_binary_and_mime_type() {
        let value = serde_json::to_value(ResourceResult::new(vec![
            ResourceContent::blob("AQID", "canvas://files/1/download").with_mime_type("image/png"),
        ]))
        .unwrap();
        assert_eq!(value["contents"][0]["mimeType"], "image/png");
        assert_eq!(value["contents"][0]["blob"], "AQID");
        assert!(value["contents"][0].get("text").is_none());
    }
    #[test]
    fn operation_names_cover_unique_reference_actions() {
        let names: std::collections::HashSet<_> = OPERATION_NAMES.iter().collect();
        assert_eq!(names.len(), 24);
        assert!(names.contains(&"attachment_download"));
        assert!(names.contains(&"create_custom_planner_item"));
    }
}
