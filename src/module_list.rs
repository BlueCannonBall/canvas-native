//! Prepare module rows when data/search/collapse state changes, not on scroll.
use crate::model::{self, Route, field};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum ModuleTarget {
    Native(Route),
    Web(String),
    None,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ModuleRow {
    Header {
        key: String,
        title: String,
        state: String,
        expanded: bool,
    },
    Item {
        key: String,
        title: String,
        kind: String,
        target: ModuleTarget,
        due: Option<String>,
        lock: Option<String>,
        completed: bool,
        indent: u32,
    },
}
struct ModuleGroup {
    key: String,
    title: String,
    search: String,
    state: String,
    items: Vec<(String, ModuleRow)>,
}
pub struct ModuleIndex {
    groups: Vec<ModuleGroup>,
}
impl ModuleIndex {
    pub fn new(course: &str, response: &Value) -> Self {
        let groups = model::list(Some(response), "modules")
            .into_iter()
            .enumerate()
            .map(|(module_index, module)| {
                let key = format!("{course}-module-{}-{module_index}", field(module, "id"));
                let title = field(module, "name");
                let items = model::list(Some(module), "items")
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let title = field(item, "title");
                        let kind = field(item, "item_type");
                        let target = if kind == "SubHeader" {
                            ModuleTarget::None
                        } else if let Some(route) = model::module_route(course, item) {
                            ModuleTarget::Native(route)
                        } else if kind == "ExternalTool" {
                            // An LTI launch must go through Canvas so the tool
                            // receives the Canvas session. The direct tool URL
                            // is not usable on its own.
                            web_target(item, &["html_url", "external_url"])
                        } else {
                            web_target(item, &["external_url", "html_url"])
                        };
                        let details = &item["content_details"];
                        let row = ModuleRow::Item {
                            key: format!("{key}-item-{}-{index}", field(item, "id")),
                            title: title.clone(),
                            kind: kind.clone(),
                            target,
                            due: details["due_at"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .map(model::date_label),
                            lock: details["lock_explanation"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .map(crate::html_content::notice_text),
                            completed: item["completion_requirement"]["completed"].as_bool()
                                == Some(true),
                            indent: item["indent"].as_u64().unwrap_or(0).min(8) as u32,
                        };
                        (format!("{title} {kind}").to_lowercase(), row)
                    })
                    .collect();
                ModuleGroup {
                    key,
                    title: title.clone(),
                    search: title.to_lowercase(),
                    state: field(module, "state").replace('_', " "),
                    items,
                }
            })
            .collect();
        Self { groups }
    }
    pub fn rows(&self, query: &str, expanded: &HashMap<String, bool>) -> Vec<ModuleRow> {
        let query = query.trim().to_lowercase();
        let mut rows = Vec::new();
        for module in &self.groups {
            let whole_module = query.is_empty() || module.search.contains(&query);
            if !whole_module
                && !module
                    .items
                    .iter()
                    .any(|(search, _)| search.contains(&query))
            {
                continue;
            }
            let open = !query.is_empty() || expanded.get(&module.key).copied().unwrap_or(true);
            rows.push(ModuleRow::Header {
                key: module.key.clone(),
                title: module.title.clone(),
                state: module.state.clone(),
                expanded: open,
            });
            if open {
                rows.extend(
                    module
                        .items
                        .iter()
                        .filter(|(search, _)| whole_module || search.contains(&query))
                        .map(|(_, row)| row.clone()),
                );
            }
        }
        rows
    }
}
fn web_target(item: &Value, keys: &[&str]) -> ModuleTarget {
    keys.iter()
        .find_map(|key| {
            item[*key]
                .as_str()
                .filter(|url| safe_url(url))
                .map(|url| ModuleTarget::Web(url.into()))
        })
        .unwrap_or(ModuleTarget::None)
}
fn safe_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https") && u.username().is_empty() && u.password().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn response() -> Value {
        json!({"modules":[{"id":"10","name":"Week 1","items":[{"id":"20","title":"Read this","item_type":"SubHeader"},{"id":"21","title":"External learning","item_type":"ExternalTool","html_url":"https://canvas.test/launch","indent":2},{"id":"22","title":"Worksheet","item_type":"Assignment","content_id":"77"}]}]})
    }
    #[test]
    fn external_links_and_subheaders_have_distinct_targets() {
        let index = ModuleIndex::new("4", &response());
        let rows = index.rows("", &HashMap::new());
        assert!(matches!(
            &rows[1],
            ModuleRow::Item {
                target: ModuleTarget::None,
                ..
            }
        ));
        assert!(
            matches!(&rows[2],ModuleRow::Item{target:ModuleTarget::Web(url),indent:2,..} if url=="https://canvas.test/launch")
        );
        assert!(
            matches!(&rows[3],ModuleRow::Item{target:ModuleTarget::Native(Route::Assignment{course,id}),..} if course=="4"&&id=="77")
        );
    }
    #[test]
    fn external_tools_launch_through_canvas_not_the_direct_tool_url() {
        let response = json!({"modules":[{"id":"10","name":"Tools","items":[
            {"title":"Codon","item_type":"ExternalTool","html_url":"https://canvas.test/courses/4/modules/items/5","external_url":"https://lti.tool.test/launch"},
            {"title":"Video","item_type":"ExternalUrl","html_url":"https://canvas.test/api/v1/courses/4/module_item_redirect/6","external_url":"https://video.test/watch"}
        ]}]});
        let index = ModuleIndex::new("4", &response);
        let rows = index.rows("", &HashMap::new());
        assert!(
            matches!(&rows[1], ModuleRow::Item { target: ModuleTarget::Web(url), .. } if url == "https://canvas.test/courses/4/modules/items/5")
        );
        assert!(
            matches!(&rows[2], ModuleRow::Item { target: ModuleTarget::Web(url), .. } if url == "https://video.test/watch")
        );
    }
    #[test]
    fn search_expands_and_retains_only_matching_items() {
        let index = ModuleIndex::new("4", &response());
        let rows = index.rows(
            "worksheet",
            &HashMap::from([("4-module-10-0".into(), false)]),
        );
        assert_eq!(rows.len(), 2);
        assert!(matches!(&rows[0], ModuleRow::Header { expanded: true, .. }));
    }
    #[test]
    fn collapse_removes_children_without_losing_module_header() {
        let index = ModuleIndex::new("4", &response());
        assert_eq!(
            index
                .rows("", &HashMap::from([("4-module-10-0".into(), false)]))
                .len(),
            1
        );
    }
    #[test]
    fn large_module_source_is_prepared_as_rows_not_elements() {
        let response = json!({"modules":[{"id":"1","name":"Large module","items":(0..5000).map(|i|json!({"id":i.to_string(),"title":format!("Entry {i}"),"item_type":"Page","page_url":format!("entry-{i}")})).collect::<Vec<_>>()}]});
        let index = ModuleIndex::new("9", &response);
        assert_eq!(index.rows("", &HashMap::new()).len(), 5001);
        assert_eq!(index.rows("Entry 4999", &HashMap::new()).len(), 2);
    }
    #[test]
    fn module_lock_notices_are_prepared_as_readable_text() {
        let index = ModuleIndex::new(
            "9",
            &json!({"modules":[{"id":"1","name":"Week","items":[{"id":"2","title":"Locked","item_type":"Assignment","content_details":{"lock_explanation":"Complete <b>Week 1</b> &amp; try again.<span hidden>browser control</span>"}}]}]}),
        );
        assert!(index.rows("",&HashMap::new()).iter().any(|row|matches!(row,ModuleRow::Item{lock:Some(text),..} if text=="Complete Week 1 & try again.")));
    }
}
