//! Prepare screen rows from cached Canvas data.
//!
//! A screen with many rows is rendered by `gpui::list`, which builds elements
//! only for the rows near the viewport. Preparing the row data is cheap; the
//! element tree is what must not be rebuilt for off-screen rows.
//!
//! These builders mirror the canvas pages they feed. Keep them in sync with the
//! row renderers in `views.rs`; unknown or missing fields stay empty rather
//! than failing the screen.

use crate::model::{self, Route};
use chrono::{DateTime, FixedOffset, NaiveDate};
use serde_json::Value;
use std::cmp::Reverse;

/// An empty search matches every row without lowercasing every title on every
/// frame; the caller already lowercases a non-empty query.
fn contains_query(value: &str, query: &str) -> bool {
    query.is_empty() || value.to_lowercase().contains(query)
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContentRow {
    AssignmentFilters,
    GradesSummary {
        current: String,
        current_grade: String,
        final_score: String,
        final_grade: String,
    },
    GradesHeader,
    FilesIntro,
    CalendarControls,
    CalendarFilters,
    CalendarRange,
    CalendarPicker,
    CalendarDayHeader {
        label: String,
    },
    Message {
        text: String,
    },
    Assignment {
        course: String,
        id: String,
        name: String,
        due: String,
        points: String,
        status: &'static str,
    },
    Announcement {
        course: String,
        id: String,
        title: String,
        author: String,
        posted: String,
        unread: bool,
    },
    Grade {
        course: String,
        id: String,
        name: String,
        due: String,
        score: String,
        status: &'static str,
    },
    /// The start of one module's files. Files screens are flattened so a
    /// module with thousands of entries is still virtualized.
    FileModule {
        name: String,
        first: bool,
    },
    File {
        course: String,
        id: String,
        title: String,
    },
    Planner(PlannerRow),
}

/// A planner item prepared for the agenda list or the dashboard sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannerRow {
    pub title: String,
    pub course_id: String,
    pub course_name: String,
    pub route: Option<Route>,
    pub url: Option<String>,
    pub date: String,
    pub status: &'static str,
    pub kind: String,
    /// Keep the original JSON value: Canvas accepts the item id in the type it
    /// sent, and the reminder write forwards it unchanged.
    pub item_id: Value,
}

impl PlannerRow {
    /// Callers pass the course label because resolving a course name needs the
    /// cached course list, not the planner item.
    pub fn new(item: &Value, course_name: String) -> Self {
        Self {
            title: model::field(item, "title"),
            course_id: model::field(item, "course_id"),
            course_name,
            route: model::planner_route(item),
            url: item["html_url"].as_str().map(str::to_owned),
            date: model::date_label(
                item["due_at"]
                    .as_str()
                    .or(item["scheduled_at"].as_str())
                    .unwrap_or(""),
            ),
            status: model::status(item),
            kind: model::field(item, "item_type"),
            item_id: item["item_id"].clone(),
        }
    }
}

pub fn assignment_rows(
    course: &str,
    data: &Value,
    query: &str,
    filter: &str,
    today: NaiveDate,
) -> Vec<ContentRow> {
    let mut items = model::list(Some(data), "assignments");
    items.sort_by_cached_key(|v| {
        let due = model::field(v, "due_at");
        (due.is_empty(), due)
    });
    items
        .into_iter()
        .filter(|v| contains_query(&model::field(v, "name"), query))
        .filter(|v| match filter {
            "Missing" => model::status(v) == "Missing",
            "Submitted" => matches!(model::status(v), "Submitted" | "Graded"),
            "Upcoming" => v["due_at"]
                .as_str()
                .and_then(model::date)
                .is_some_and(|d| d >= today),
            _ => true,
        })
        .map(|assignment| ContentRow::Assignment {
            course: course.into(),
            id: model::field(assignment, "id"),
            name: model::field(assignment, "name"),
            due: model::date_label(assignment["due_at"].as_str().unwrap_or("")),
            points: model::field(assignment, "points_possible"),
            status: model::status(assignment),
        })
        .collect()
}

fn announcement_posted_at(value: &Value) -> &str {
    ["posted_at", "delayed_post_at", "created_at"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .unwrap_or("")
}

fn announcement_time(value: &Value) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(announcement_posted_at(value)).ok()
}

/// Announcements are newest first, matching the course Announcements page.
pub fn announcement_rows(course: &str, data: &Value, query: &str) -> Vec<ContentRow> {
    let mut items = model::list(Some(data), "announcements");
    items.sort_by_cached_key(|value| Reverse(announcement_time(value)));
    items
        .into_iter()
        .filter(|value| contains_query(&model::field(value, "title"), query))
        .map(|announcement| {
            let posted_at = announcement_posted_at(announcement);
            ContentRow::Announcement {
                course: course.into(),
                id: model::field(announcement, "id"),
                title: model::field(announcement, "title"),
                author: model::field(&announcement["author"], "display_name"),
                posted: if posted_at.is_empty() {
                    String::new()
                } else {
                    model::date_label(posted_at)
                },
                unread: model::field(announcement, "read_state") == "unread"
                    || announcement["unread_count"]
                        .as_u64()
                        .is_some_and(|count| count > 0),
            }
        })
        .collect()
}

pub fn grade_rows(course: &str, grades: &Value, query: &str) -> Vec<ContentRow> {
    model::list(Some(grades), "assignments")
        .into_iter()
        .filter(|grade| contains_query(&model::field(grade, "name"), query))
        .map(|grade| ContentRow::Grade {
            course: course.into(),
            id: model::field(grade, "assignment_id"),
            name: model::field(grade, "name"),
            due: model::date_label(grade["due_at"].as_str().unwrap_or("")),
            score: if grade["score"].is_null() {
                "—".into()
            } else {
                format!(
                    "{} / {}",
                    model::field(grade, "score"),
                    model::field(grade, "points_possible")
                )
            },
            status: model::status(grade),
        })
        .collect()
}

pub fn file_rows(course: &str, data: &Value, query: &str) -> Vec<ContentRow> {
    let mut rows = Vec::new();
    for module in model::list(Some(data), "modules") {
        let files: Vec<_> = model::list(Some(module), "items")
            .into_iter()
            .filter(|v| model::field(v, "item_type") == "File")
            .filter(|v| contains_query(&model::field(v, "title"), query))
            .collect();
        if files.is_empty() {
            continue;
        }
        let first = !rows
            .iter()
            .any(|row| matches!(row, ContentRow::FileModule { .. }));
        rows.push(ContentRow::FileModule {
            name: model::field(module, "name"),
            first,
        });
        for file in files {
            rows.push(ContentRow::File {
                course: course.into(),
                id: model::field(file, "content_id"),
                title: model::field(file, "title"),
            });
        }
    }
    rows
}

/// Agenda items for one day, or for the whole selected range when `day` is
/// `None`, in due-date order.
pub fn agenda_rows<'a>(
    items: impl IntoIterator<Item = &'a Value>,
    day: Option<NaiveDate>,
    query: &str,
    course_name: impl Fn(&str) -> String,
) -> Vec<ContentRow> {
    let mut items: Vec<_> = items
        .into_iter()
        .filter(|item| day.is_none_or(|day| model::planner_date(item) == Some(day)))
        .filter(|item| contains_query(&model::field(item, "title"), query))
        .collect();
    items.sort_by_cached_key(|item| model::planner_date(item));
    items
        .into_iter()
        .map(|item| {
            let course = model::field(item, "course_id");
            let name = if course.is_empty() {
                "Personal to-do".into()
            } else {
                course_name(&course)
            };
            ContentRow::Planner(PlannerRow::new(item, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn day(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn assignment_rows_follow_the_filter_and_keep_due_order() {
        let data = json!({"assignments": [
            {"id": "1", "name": "Later", "due_at": "2026-11-20T18:00:00Z", "points_possible": 10, "submission": {"missing": false}},
            {"id": "2", "name": "Sooner", "due_at": "2026-11-12T18:00:00Z", "points_possible": 20, "submission": {"missing": true}},
            {"id": "3", "name": "Undated", "points_possible": 30, "submission": {"missing": false}},
        ]});
        let rows = assignment_rows("4", &data, "", "All", day("2026-11-08"));
        assert_eq!(rows.len(), 3);
        assert!(matches!(&rows[0], ContentRow::Assignment { name, .. } if name == "Sooner"));
        assert!(matches!(&rows[2], ContentRow::Assignment { name, .. } if name == "Undated"));

        let missing = assignment_rows("4", &data, "", "Missing", day("2026-11-08"));
        assert_eq!(missing.len(), 1);
        assert!(
            matches!(&missing[0], ContentRow::Assignment { id, status, .. } if id == "2" && *status == "Missing")
        );

        let searched = assignment_rows("4", &data, "later", "All", day("2026-11-08"));
        assert_eq!(searched.len(), 1);
    }

    #[test]
    fn announcement_rows_are_newest_first_and_searchable() {
        let data = json!({"announcements": [
            {"id": "1", "title": "Older update", "posted_at": "2026-11-01T12:00:00Z", "author": {"display_name": "Dr. Chen"}},
            {"id": "2", "title": "Lab moved", "posted_at": "2026-11-06T12:00:00Z", "author": {"display_name": "Dr. Chen"}, "read_state": "unread"},
            {"id": "3", "title": "Undated notice", "author": {"display_name": "Registrar"}},
        ]});
        let rows = announcement_rows("42", &data, "");
        assert_eq!(rows.len(), 3);
        assert!(
            matches!(&rows[0], ContentRow::Announcement { id, posted, unread, .. } if id == "2" && !posted.is_empty() && *unread)
        );
        assert!(matches!(&rows[1], ContentRow::Announcement { id, .. } if id == "1"));
        assert!(
            matches!(&rows[2], ContentRow::Announcement { id, posted, .. } if id == "3" && posted.is_empty()),
            "announcements without a post date stay last"
        );

        let searched = announcement_rows("42", &data, "lab");
        assert_eq!(searched.len(), 1);
        assert!(
            matches!(&searched[0], ContentRow::Announcement { author, .. } if author == "Dr. Chen")
        );
    }

    #[test]
    fn file_rows_flatten_modules_and_skip_empty_ones() {
        let data = json!({"modules": [
            {"id": "1", "name": "Empty", "items": [{"title": "Page", "item_type": "Page"}]},
            {"id": "2", "name": "Readings", "items": [
                {"title": "Syllabus.pdf", "item_type": "File", "content_id": "9"},
                {"title": "Notes", "item_type": "Page"},
            ]},
        ]});
        let rows = file_rows("4", &data, "");
        assert_eq!(rows.len(), 2);
        assert!(
            matches!(&rows[0], ContentRow::FileModule { name, first: true } if name == "Readings")
        );
        assert!(matches!(&rows[1], ContentRow::File { id, .. } if id == "9"));
        assert!(file_rows("4", &data, "missing").is_empty());
    }

    #[test]
    fn agenda_rows_filter_by_day_and_resolve_course_labels() {
        let items = vec![
            json!({"item_id": "1", "item_type": "assignment", "course_id": "4", "title": "Essay", "due_at": "2026-11-20T18:00:00Z"}),
            json!({"item_id": "2", "item_type": "planner_note", "title": "Personal", "scheduled_at": "2026-11-12"}),
        ];
        let rows = agenda_rows(&items, None, "", |course| format!("Course {course}"));
        assert_eq!(rows.len(), 2);
        assert!(
            matches!(&rows[0], ContentRow::Planner(row) if row.title == "Personal" && row.course_name == "Personal to-do")
        );
        assert!(matches!(&rows[1], ContentRow::Planner(row) if row.course_name == "Course 4"));

        let one_day = agenda_rows(&items, Some(day("2026-11-12")), "", |_| String::new());
        assert_eq!(one_day.len(), 1);
    }
}
