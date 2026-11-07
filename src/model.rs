//! Navigation and display rules. Canvas identifiers never become user input.
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CourseTab {
    Home,
    Modules,
    Assignments,
    Grades,
    Files,
    Syllabus,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocumentView {
    #[default]
    Pages,
    Text,
}
#[derive(Clone, Debug)]
pub struct DocumentSelection {
    pub course: Option<String>,
    pub resource: String,
    pub title: String,
    pub start: u64,
}
/// Nominal reader column width until the page list has been laid out. At 100%
/// zoom a page fits this column; the fetched raster is twice as wide.
pub const DOCUMENT_PAGE_WIDTH: f32 = 900.0;
/// How many pages beyond the current one the reader prepares in the background.
pub const DOCUMENT_PREFETCH_AHEAD: u32 = 2;
/// Zoom bounds wide enough for a full page to fit a short window.
pub const DOCUMENT_MIN_ZOOM: u32 = 25;
pub const DOCUMENT_MAX_ZOOM: u32 = 200;

pub fn document_preview_request(resource: &str, page: u32, zoom: u32) -> Request {
    let raster =
        (crate::document_preview::DEFAULT_RASTER_SIZE.saturating_mul(zoom) / 100).clamp(400, 2800);
    Request::new(
        "document_preview",
        json!({"resource":resource,"page":page.max(1),"raster_size":raster}),
    )
}

/// Pages the reader should have in memory or in flight around `current`.
pub fn document_page_range(current: u32, pages: u32) -> std::ops::RangeInclusive<u32> {
    let pages = pages.max(1);
    let first = current.clamp(1, pages);
    let last = first.saturating_add(DOCUMENT_PREFETCH_AHEAD).min(pages);
    first..=last
}
impl CourseTab {
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Modules => "Modules",
            Self::Assignments => "Assignments",
            Self::Grades => "Grades",
            Self::Files => "Files",
            Self::Syllabus => "Syllabus",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Dashboard,
    Courses,
    Calendar,
    Grades,
    Account,
    Course {
        id: String,
        tab: CourseTab,
    },
    Assignment {
        course: String,
        id: String,
    },
    Page {
        course: String,
        slug: String,
    },
    File {
        course: String,
        id: String,
    },
    Document {
        course: Option<String>,
        resource: String,
        title: String,
        start: u64,
    },
    Note(String),
}
impl Route {
    pub fn course_id(&self) -> Option<&str> {
        match self {
            Self::Course { id, .. } => Some(id),
            Self::Assignment { course, .. }
            | Self::Page { course, .. }
            | Self::File { course, .. } => Some(course),
            Self::Document { course, .. } => course.as_deref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub name: String,
    pub args: Value,
}
impl Request {
    pub fn new(name: &str, args: Value) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}

pub fn cache_key(name: &str, args: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by_key(|(k, _)| *k);
                let mut result = serde_json::Map::new();
                for (k, v) in entries {
                    result.insert(k.clone(), canonical(v));
                }
                Value::Object(result)
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            _ => value.clone(),
        }
    }
    format!("{name}:{}", canonical(args))
}
pub fn requests(route: &Route) -> Vec<Request> {
    match route {
        Route::Dashboard | Route::Courses => vec![Request::new("course_list", json!({}))],
        Route::Grades => vec![Request::new("grade_summary", json!({}))],
        Route::Course { id, tab } => vec![Request::new(
            match tab {
                CourseTab::Home | CourseTab::Syllabus => "course_info",
                CourseTab::Modules | CourseTab::Files => "module_list",
                CourseTab::Assignments => "assignment_list",
                CourseTab::Grades => "course_grades",
            },
            json!({"course_id":id}),
        )],
        // assignment_info already includes the current user's submission and
        // grade. Do not make another course/grade round trip just to show it.
        Route::Assignment { course, id } => vec![Request::new(
            "assignment_info",
            json!({"course_id":course,"assignment_id":id}),
        )],
        Route::Page { course, slug } => vec![Request::new(
            "page_info",
            json!({"course_id":course,"page_url":slug}),
        )],
        Route::File { id, .. } => vec![Request::new("file_info", json!({"file_id":id}))],
        Route::Document {
            resource, start, ..
        } => vec![Request::new(
            "attachment_text",
            json!({"resource":resource,"start":start}),
        )],
        Route::Note(id) => vec![Request::new("planner_note_info", json!({"note_id":id}))],
        _ => vec![],
    }
}
/// Canvas chooses a landing page per course; Home is not always Syllabus.
pub fn course_home_view(course: &Value) -> &str {
    match course["default_view"].as_str() {
        Some("modules") => "modules",
        Some("assignments") => "assignments",
        Some("wiki") => "wiki",
        Some("syllabus") => "syllabus",
        Some("feed") => "feed",
        _ => "overview",
    }
}
pub fn course_home_request(id: &str, course: &Value) -> Option<Request> {
    let name = match course_home_view(course) {
        "modules" => "module_list",
        "assignments" => "assignment_list",
        "wiki" => "course_front_page",
        _ => return None,
    };
    Some(Request::new(name, json!({"course_id":id})))
}
/// Warm only the course the user has opened, never every enrolled course and
/// never attachment bodies, grade reports, authentication, or writes.
pub fn course_prefetch(route: &Route) -> Vec<Request> {
    let Some(id) = route.course_id() else {
        return vec![];
    };
    ["course_info", "module_list", "assignment_list"]
        .into_iter()
        .map(|name| Request::new(name, json!({"course_id":id})))
        .collect()
}
pub fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .filter(|v| !v.is_null())
        .map(display)
        .unwrap_or_default()
}
pub fn display(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
pub fn list<'a>(value: Option<&'a Value>, key: &str) -> Vec<&'a Value> {
    value
        .and_then(|v| v[key].as_array())
        .map(|v| v.iter().collect())
        .unwrap_or_default()
}
pub fn course_title(course: &Value) -> String {
    let name = field(course, "name");
    if name.is_empty() {
        "Untitled course".into()
    } else {
        name
    }
}
pub fn course_color(id: &str) -> u32 {
    const COLORS: [u32; 6] = [0x267b96, 0x8055a2, 0x25836b, 0xc16b30, 0x386bb3, 0xa64a62];
    let n = id
        .bytes()
        .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
    COLORS[n % COLORS.len()]
}
pub fn course_is_current(course: &Value) -> bool {
    course["concluded"].as_bool() != Some(true)
        && !matches!(
            course["workflow_state"].as_str(),
            Some("completed" | "deleted")
        )
}
pub fn date(value: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|d| d.with_timezone(&Local).date_naive())
        .or_else(|| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
}

pub fn planner_date(item: &Value) -> Option<NaiveDate> {
    item["due_at"]
        .as_str()
        .or(item["scheduled_at"].as_str())
        .and_then(date)
}

pub fn planner_items_by_date<'a>(
    items: impl IntoIterator<Item = &'a Value>,
) -> std::collections::BTreeMap<NaiveDate, Vec<&'a Value>> {
    let mut days = std::collections::BTreeMap::<_, Vec<_>>::new();
    for item in items {
        if let Some(day) = planner_date(item) {
            days.entry(day).or_default().push(item);
        }
    }
    days
}
pub fn date_label(value: &str) -> String {
    if value.is_empty() {
        return "No due date".into();
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(value) {
        return d
            .with_timezone(&Local)
            .format("%b %-d at %-I:%M %p")
            .to_string();
    }
    date(value)
        .map(|d| d.format("%b %-d, %Y").to_string())
        .unwrap_or_else(|| value.to_owned())
}

pub fn scheduled_at(date: &str, time: &str) -> anyhow::Result<String> {
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("Choose a valid date (YYYY-MM-DD)."))?;
    if time.trim().is_empty() {
        return Ok(date.to_string());
    }
    let time = NaiveTime::parse_from_str(time.trim(), "%H:%M")
        .map_err(|_| anyhow::anyhow!("Enter a time in 24-hour format, such as 14:30."))?;
    Local
        .from_local_datetime(&date.and_time(time))
        .single()
        .map(|value| value.to_rfc3339())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "This local time falls within a daylight-saving change. Choose another time."
            )
        })
}
pub fn status(value: &Value) -> &'static str {
    let s = value
        .get("submission")
        .filter(|s| !s.is_null())
        .unwrap_or(value);
    if s["excused"].as_bool() == Some(true) {
        "Excused"
    } else if s["missing"].as_bool() == Some(true) {
        "Missing"
    } else if s["late"].as_bool() == Some(true) {
        "Late"
    } else if s["graded"].as_bool() == Some(true)
        || s["workflow_state"] == "graded"
        || s["status"] == "graded"
        || s["submission_status"] == "graded"
    {
        "Graded"
    } else if s["submitted"].as_bool() == Some(true) || s["workflow_state"] == "submitted" {
        "Submitted"
    } else if s["marked_complete"].as_bool() == Some(true) {
        "Completed"
    } else {
        "Not submitted"
    }
}
pub fn score(value: &Value) -> String {
    match value.as_f64() {
        Some(n) => format!("{n:.1}%"),
        None => "Not graded".into(),
    }
}
pub fn month_start(day: NaiveDate) -> NaiveDate {
    day.with_day(1).unwrap()
}
pub fn shift_month(day: NaiveDate, delta: i32) -> NaiveDate {
    let index = day.year() * 12 + day.month0() as i32 + delta;
    NaiveDate::from_ymd_opt(index.div_euclid(12), index.rem_euclid(12) as u32 + 1, 1).unwrap()
}
pub fn module_route(course: &str, item: &Value) -> Option<Route> {
    let id = field(item, "content_id");
    match field(item, "item_type").as_str() {
        "Assignment" if !id.is_empty() => Some(Route::Assignment {
            course: course.into(),
            id,
        }),
        "File" if !id.is_empty() => Some(Route::File {
            course: course.into(),
            id,
        }),
        "Page" if !field(item, "page_url").is_empty() => Some(Route::Page {
            course: course.into(),
            slug: field(item, "page_url"),
        }),
        _ => None,
    }
}
pub fn planner_route(item: &Value) -> Option<Route> {
    let id = field(item, "item_id");
    let course = field(item, "course_id");
    match field(item, "item_type").as_str() {
        "assignment" if !course.is_empty() && !id.is_empty() => {
            Some(Route::Assignment { course, id })
        }
        "planner_note" if !id.is_empty() => Some(Route::Note(id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn course_home_follows_canvas_settings_not_syllabus_content() {
        for (view, name) in [
            ("modules", "module_list"),
            ("assignments", "assignment_list"),
            ("wiki", "course_front_page"),
        ] {
            let course =
                serde_json::json!({"default_view":view,"syllabus_body_html":"<h1>Syllabus</h1>"});
            let request = super::course_home_request("123", &course).unwrap();
            assert_eq!(request.name, name);
            assert_eq!(request.args, serde_json::json!({"course_id":"123"}));
            assert_eq!(super::course_home_view(&course), view);
        }
        for view in ["syllabus", "feed", "unknown"] {
            assert!(
                super::course_home_request("123", &serde_json::json!({"default_view":view}))
                    .is_none()
            );
        }
        assert_eq!(super::course_home_view(&serde_json::json!({})), "overview");
    }
    use super::*;
    #[test]
    fn prefetch_is_bounded_to_read_only_metadata_for_the_open_course() {
        assert!(course_prefetch(&Route::Dashboard).is_empty());
        let requests = course_prefetch(&Route::Course {
            id: "42".into(),
            tab: CourseTab::Home,
        });
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|r| r.args["course_id"] == "42"
            && matches!(
                r.name.as_str(),
                "course_info" | "module_list" | "assignment_list"
            )));
    }
    #[test]
    fn preview_keys_distinguish_page_and_zoom_and_bound_raster_work() {
        let a = document_preview_request("canvas://files/4/download", 1, 100);
        let b = document_preview_request("canvas://files/4/download", 2, 100);
        let c = document_preview_request("canvas://files/4/download", 1, 150);
        assert_ne!(cache_key(&a.name, &a.args), cache_key(&b.name, &b.args));
        assert_ne!(cache_key(&a.name, &a.args), cache_key(&c.name, &c.args));
        assert_eq!(
            document_preview_request("canvas://files/4/download", 0, u32::MAX).args["page"],
            1
        );
        assert!(
            document_preview_request("canvas://files/4/download", 1, u32::MAX).args["raster_size"]
                .as_u64()
                .unwrap()
                <= 2800
        );
    }
    #[test]
    fn prefetch_stays_inside_the_document_and_covers_the_next_pages() {
        assert_eq!(document_page_range(1, 10), 1..=3);
        assert_eq!(document_page_range(9, 10), 9..=10);
        assert_eq!(document_page_range(1, 1), 1..=1);
        assert_eq!(document_page_range(0, 10), 1..=3);
        assert_eq!(document_page_range(99, 10), 10..=10);
        assert_eq!(DOCUMENT_PAGE_WIDTH, 900.0);
    }
    #[test]
    fn course_navigation_carries_identifiers_without_user_entry() {
        let route = Route::Course {
            id: "7391".into(),
            tab: CourseTab::Assignments,
        };
        assert_eq!(requests(&route)[0].args, json!({"course_id":"7391"}));
    }
    #[test]
    fn assignment_screen_uses_the_submission_in_its_single_detail_request() {
        let requests = requests(&Route::Assignment {
            course: "4".into(),
            id: "8".into(),
        });
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].name, "assignment_info");
        assert_eq!(
            requests[0].args,
            json!({"course_id":"4","assignment_id":"8"})
        );
    }
    #[test]
    fn module_and_planner_items_open_the_actual_selected_item() {
        assert_eq!(
            module_route("71", &json!({"item_type":"Assignment","content_id":"92"})),
            Some(Route::Assignment {
                course: "71".into(),
                id: "92".into()
            })
        );
        assert_eq!(
            planner_route(&json!({"item_type":"planner_note","item_id":"83"})),
            Some(Route::Note("83".into()))
        );
    }
    #[test]
    fn absent_grades_are_not_zero() {
        assert_eq!(score(&Value::Null), "Not graded");
        assert_eq!(score(&json!(0)), "0.0%");
    }
    #[test]
    fn cache_key_does_not_depend_on_argument_order() {
        assert_eq!(
            cache_key("x", &json!({"a":1,"b":2})),
            cache_key("x", &json!({"b":2,"a":1}))
        );
    }
    #[test]
    fn months_cross_year_boundaries() {
        assert_eq!(
            shift_month(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(), 1),
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );
    }
    #[test]
    fn todo_dates_and_optional_local_times_are_validated() {
        assert_eq!(scheduled_at("2026-11-12", "").unwrap(), "2026-11-12");
        assert!(
            DateTime::parse_from_rfc3339(&scheduled_at("2026-11-12", "14:30").unwrap()).is_ok()
        );
        assert!(scheduled_at("2026-11-12", "25:00").is_err());
        assert!(scheduled_at("2026-02-30", "").is_err());
    }

    #[test]
    fn calendar_groups_due_and_scheduled_items_once_before_rendering_days() {
        let items = [
            json!({"title":"Assignment", "due_at":"2026-11-12"}),
            json!({"title":"Note", "scheduled_at":"2026-11-12"}),
            json!({"title":"Other", "due_at":"2026-11-13"}),
            json!({"title":"Undated"}),
        ];
        let days = planner_items_by_date(&items);
        assert_eq!(days.len(), 2);
        assert_eq!(
            days[&NaiveDate::from_ymd_opt(2026, 11, 12).unwrap()].len(),
            2
        );
        assert_eq!(days.values().map(Vec::len).sum::<usize>(), 3);
    }
}
