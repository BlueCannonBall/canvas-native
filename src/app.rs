use crate::module_list::{ModuleIndex, ModuleRow};
use crate::{
    backend::{Backend, Operation, Settings},
    model::{self, CourseTab, DocumentSelection, DocumentView, Request, Route, field},
    schema,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{Datelike, Local, NaiveDate};
use gpui::{
    Bounds, Entity, Image, ImageFormat, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowOptions, prelude::*, px, size,
};
use gpui::{ListAlignment, ListState};
use gpui_component::Root;
use gpui_component::input::InputState;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};

#[path = "views.rs"]
mod views;

#[derive(Clone, Copy, PartialEq)]
enum Picker {
    CalendarCourse,
    NoteCourse,
    LinkedItem,
}
struct NoteEditor {
    id: Option<String>,
    title: Entity<InputState>,
    details: Entity<InputState>,
    date: Entity<InputState>,
    time: Entity<InputState>,
    course: Option<String>,
    linked: Option<Value>,
}
pub struct CanvasApp {
    backend: Backend,
    runtime: tokio::runtime::Handle,
    settings: Settings,
    demo: bool,
    /// A reader window owns no navigation chrome: it only renders the document.
    standalone_document: bool,
    /// A file row click fetches file metadata here before opening a reader
    /// window. The main window stays on the list it was browsing.
    pending_document_file: Option<(String, String)>,
    route: Route,
    history: Vec<Route>,
    courses_open: bool,
    courses_focus: gpui::FocusHandle,
    courses_previous_focus: Option<gpui::FocusHandle>,
    data: HashMap<String, Value>,
    failures: HashMap<String, String>,
    queue: VecDeque<Operation>,
    pending: Option<Operation>,
    pending_id: Option<u64>,
    next_request_id: u64,
    session_generation: u64,
    prefetch_pending: HashMap<String, u64>,
    prefetch_queue: VecDeque<Operation>,
    prefetch_attempted: HashSet<String>,
    error: Option<String>,
    status: String,
    updated: Option<String>,
    search: Entity<InputState>,
    origin: Entity<InputState>,
    month: NaiveDate,
    calendar_course: Option<String>,
    calendar_filter: Option<String>,
    calendar_list: bool,
    range_start: Entity<InputState>,
    range_end: Entity<InputState>,
    applied_range: Option<(String, String)>,
    calendar_day: Option<NaiveDate>,
    picker: Option<Picker>,
    note: Option<NoteEditor>,
    confirm_delete: Option<String>,
    confirm_save: bool,
    show_past: bool,
    page: usize,
    assignment_filter: String,
    expanded_modules: HashMap<String, bool>,
    module_indexes: HashMap<String, ModuleIndex>,
    module_rows: Arc<Vec<ModuleRow>>,
    module_rows_course: String,
    module_rows_query: String,
    module_scroll: ListState,
    last_saved: Option<PathBuf>,
    image: Option<Arc<Image>>,
    image_content: Option<Value>,
    resource_images: HashMap<String, Arc<Image>>,
    document_view: DocumentView,
    document_resource: String,
    document_page: u32,
    document_zoom: u32,
    document_jump: Entity<InputState>,
    document_jump_reset: bool,
    document_scroll: ListState,
    document_images: HashMap<String, Arc<Image>>,
    document_image_order: VecDeque<String>,
    show_technical: bool,
    window_title: String,
    renderer: Option<gpui::GpuSpecs>,
    _subscriptions: Vec<Subscription>,
}
fn input(
    value: &str,
    placeholder: &str,
    multiline: bool,
    window: &mut Window,
    cx: &mut Context<CanvasApp>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .placeholder(placeholder.to_owned())
            .multi_line(multiline);
        state.set_value(value.to_owned(), window, cx);
        state
    })
}
impl CanvasApp {
    pub fn new(
        settings: Settings,
        runtime: &tokio::runtime::Handle,
        demo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(settings, runtime, demo, None, None, window, cx)
    }
    /// A second window that only shows a document. It shares the Canvas
    /// backend (and its cached previews) with the window that opened it, so a
    /// file opens where it was clicked without re-authenticating.
    fn new_document(
        settings: Settings,
        runtime: &tokio::runtime::Handle,
        backend: Backend,
        demo: bool,
        document: DocumentSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(
            settings,
            runtime,
            demo,
            Some(backend),
            Some(document),
            window,
            cx,
        )
    }
    fn build(
        settings: Settings,
        runtime: &tokio::runtime::Handle,
        demo: bool,
        shared_backend: Option<Backend>,
        document: Option<DocumentSelection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let standalone_document = document.is_some();
        let search = input("", "Search…", false, window, cx);
        let subscription = cx.observe(&search, |this, _, cx| {
            this.page = 0;
            this.rebuild_module_rows(cx);
            cx.notify();
        });
        let today = Local::now().date_naive();
        settings.appearance.apply(Some(window), cx);
        let appearance_subscription = cx.observe_window_appearance(window, |this, window, cx| {
            if this.settings.appearance == crate::appearance::Appearance::System {
                this.settings.appearance.apply(Some(window), cx);
                cx.notify();
            }
        });
        let configured = demo || settings.validate().is_ok();
        let route = if let Some(document) = document {
            Route::Document {
                course: document.course,
                resource: document.resource,
                title: document.title,
                start: document.start,
            }
        } else if demo {
            crate::demo::initial_route()
        } else if configured {
            Route::Dashboard
        } else {
            Route::Account
        };
        let mut this = Self {
            backend: shared_backend.unwrap_or_else(|| Backend::start(settings.clone(), runtime)),
            runtime: runtime.clone(),
            origin: input(
                &settings.origin,
                "https://canvas.your-university.edu",
                false,
                window,
                cx,
            ),
            settings,
            demo,
            standalone_document,
            pending_document_file: None,
            route,
            history: vec![],
            courses_open: false,
            courses_focus: cx.focus_handle(),
            courses_previous_focus: None,
            data: HashMap::new(),
            failures: HashMap::new(),
            queue: VecDeque::new(),
            pending: None,
            pending_id: None,
            next_request_id: 0,
            session_generation: 0,
            prefetch_pending: HashMap::new(),
            prefetch_queue: VecDeque::new(),
            prefetch_attempted: HashSet::new(),
            error: None,
            status: if demo {
                "Demo · sample courses".into()
            } else {
                "Ready".into()
            },
            updated: None,
            search,
            month: model::month_start(today),
            calendar_course: None,
            calendar_filter: None,
            calendar_list: false,
            range_start: input(
                &model::month_start(today).to_string(),
                "YYYY-MM-DD",
                false,
                window,
                cx,
            ),
            range_end: input(
                &(model::shift_month(today, 1) - chrono::Duration::days(1)).to_string(),
                "YYYY-MM-DD",
                false,
                window,
                cx,
            ),
            applied_range: None,
            calendar_day: None,
            picker: None,
            note: None,
            confirm_delete: None,
            confirm_save: false,
            show_past: false,
            page: 0,
            assignment_filter: "All".into(),
            expanded_modules: HashMap::new(),
            module_indexes: HashMap::new(),
            module_rows: Arc::new(Vec::new()),
            module_rows_course: String::new(),
            module_rows_query: String::new(),
            module_scroll: ListState::new(0, ListAlignment::Top, px(400.)),
            last_saved: None,
            image: None,
            image_content: None,
            resource_images: HashMap::new(),
            document_view: DocumentView::Pages,
            document_resource: String::new(),
            document_page: 1,
            document_zoom: 100,
            document_jump: input("1", "Page", false, window, cx),
            document_jump_reset: false,
            document_scroll: ListState::new(0, ListAlignment::Top, px(400.)),
            document_images: HashMap::new(),
            document_image_order: VecDeque::new(),
            show_technical: false,
            window_title: String::new(),
            renderer: window.gpu_specs(),
            _subscriptions: vec![subscription, appearance_subscription],
        };
        let scroll = this.document_scroll.clone();
        let weak = cx.entity().downgrade();
        scroll.set_scroll_handler(move |event, _window, cx| {
            let visible = event.visible_range.clone();
            let _ = weak.update(cx, |this, cx| this.document_scrolled(visible, cx));
        });
        if standalone_document {
            this.load_route(cx);
        } else if configured {
            this.enqueue_read(Request::new("course_list", json!({})), false);
            this.load_route(cx);
        }
        this
    }
    fn get(&self, name: &str, args: Value) -> Option<&Value> {
        self.data.get(&model::cache_key(name, &args))
    }
    fn courses(&self) -> Vec<&Value> {
        model::list(self.get("course_list", json!({})), "courses")
    }
    fn course_name(&self, id: &str) -> String {
        self.courses()
            .into_iter()
            .find(|c| field(c, "id") == id)
            .map(model::course_title)
            .or_else(|| {
                self.get("course_info", json!({"course_id":id}))
                    .map(|v| model::course_title(&v["course"]))
            })
            .unwrap_or_else(|| "Course".into())
    }
    fn go(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        // A file the user has already navigated away from should no longer pop
        // open a reader when its metadata finally arrives.
        self.pending_document_file = None;
        if self.courses_open {
            self.close_courses(window, cx);
        }
        self.courses_open = false;
        // Module file rows resolve to Route::File only to discover the Canvas
        // resource. Keep the module list visible and open the reader window.
        if let Route::File { course, id } = &route {
            self.open_file_window(course.clone(), id.clone(), cx);
            return;
        }
        if self.route.course_id() != route.course_id() {
            self.prefetch_queue.clear();
        }
        if self.route != route {
            self.history.push(self.route.clone());
            if self.history.len() > 30 {
                self.history.remove(0);
            }
            self.route = route;
        }
        self.search
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.page = 0;
        self.error = None;
        self.picker = None;
        self.image = None;
        self.image_content = None;
        // A response is stored by its request key, never substituted into the new screen.
        self.queue.retain(
            |operation| !matches!(operation,Operation::Tool{name,..}if !schema::is_write(name)),
        );
        self.load_route(cx);
        self.rebuild_module_rows(cx);
        cx.notify();
    }
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(route) = self.history.pop() {
            self.route = route;
            self.search.update(cx, |s, cx| s.set_value("", window, cx));
            self.load_route(cx);
            self.rebuild_module_rows(cx);
            cx.notify();
        }
    }
    fn rebuild_module_rows(&mut self, cx: &mut Context<Self>) {
        if !self.showing_modules() {
            return;
        }
        let Route::Course { id, .. } = &self.route else {
            return;
        };
        let query = self.search.read(cx).value().to_string();
        let rows = self
            .module_indexes
            .get(id)
            .map(|index| index.rows(&query, &self.expanded_modules))
            .unwrap_or_default();
        let reset = self.module_rows_course != *id || self.module_rows_query != query;
        if !reset && *self.module_rows == rows {
            return;
        }
        if reset {
            self.module_scroll.reset(rows.len());
        } else {
            let prefix = self
                .module_rows
                .iter()
                .zip(&rows)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = self
                .module_rows
                .iter()
                .rev()
                .zip(rows.iter().rev())
                .take(self.module_rows.len().min(rows.len()) - prefix)
                .take_while(|(a, b)| a == b)
                .count();
            self.module_scroll.splice(
                prefix..self.module_rows.len() - suffix,
                rows.len() - prefix - suffix,
            );
        }
        self.module_rows_course = id.clone();
        self.module_rows_query = query;
        self.module_rows = Arc::new(rows);
    }
    fn close_courses(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.courses_open = false;
        if let Some(focus) = self.courses_previous_focus.take() {
            window.focus(&focus);
        }
        cx.notify();
    }
    fn clear_module_cache(&mut self) {
        self.module_indexes.clear();
        self.module_rows = Arc::new(Vec::new());
        self.module_rows_course.clear();
        self.module_rows_query.clear();
        self.module_scroll.reset(0);
    }
    fn planner_request(&self, dashboard: bool) -> Request {
        if dashboard {
            return Request::new("dashboard_items", json!({"filter":"incomplete_items"}));
        }
        let mut args = if let Some((start, end)) = &self.applied_range {
            json!({"start_date":start,"end_date":end})
        } else {
            let start = if self.calendar_list {
                self.month
            } else {
                self.month
                    - chrono::Duration::days(self.month.weekday().num_days_from_sunday() as i64)
            };
            let end = if self.calendar_list {
                model::shift_month(self.month, 1) - chrono::Duration::days(1)
            } else {
                start + chrono::Duration::days(41)
            };
            json!({"start_date":start.to_string(),"end_date":end.to_string()})
        };
        if let Some(course) = &self.calendar_course {
            args["course_ids"] = json!([course]);
        }
        if let Some(filter) = &self.calendar_filter {
            args["filter"] = json!(filter);
        }
        Request::new("dashboard_items", args)
    }
    fn load_route(&mut self, cx: &mut Context<Self>) {
        self.prepare_document();
        let mut requests = self.route_requests();
        if matches!(self.route, Route::Dashboard) {
            requests.push(self.planner_request(true));
        }
        if matches!(self.route, Route::Calendar) {
            requests.push(self.planner_request(false));
        }
        let needed: HashSet<_> = requests
            .iter()
            .map(|r| model::cache_key(&r.name, &r.args))
            .collect();
        self.queue.retain(|op|!matches!(op,Operation::Tool{name,..}if matches!(name.as_str(),"document_preview"|"attachment_text"))||operation_key(op).is_some_and(|k|needed.contains(&k)));
        if let Some(Operation::Tool { name, .. }) = &self.pending
            && crate::backend::is_cacheable_read(name)
            && self
                .pending
                .as_ref()
                .and_then(operation_key)
                .is_some_and(|key| !needed.contains(&key))
        {
            let old = self.pending.take().expect("pending read");
            let id = self.pending_id.take().expect("pending read id");
            if self.prefetch_pending.len() >= 3 {
                // Bound obsolete navigation work. These are reads only; never
                // cancel/replay an authentication or Canvas write operation.
                if self.standalone_document {
                    // A reader window shares the backend with the main window;
                    // cancelling here would also fail the visible app's reads.
                    self.prefetch_queue.clear();
                } else {
                    self.backend.cancel();
                    self.prefetch_pending.clear();
                    self.prefetch_queue.clear();
                }
            } else if let Some(key) = operation_key(&old) {
                self.prefetch_pending.insert(key, id);
            }
        }
        for request in requests {
            self.enqueue_read(request, false);
        }
        if self.active_document().is_some() && self.document_view == DocumentView::Pages {
            self.sync_document_pages(cx);
        }
        self.pump(cx);
    }
    fn active_document(&self) -> Option<DocumentSelection> {
        match &self.route {
            Route::Document {
                course,
                resource,
                title,
                start,
            } => Some(DocumentSelection {
                course: course.clone(),
                resource: resource.clone(),
                title: title.clone(),
                start: *start,
            }),
            Route::File { course, id } => {
                let file = &self.get("file_info", json!({"file_id":id}))?["file"];
                let resource = file["resource"]
                    .as_str()
                    .or(file["download_resource"].as_str())
                    .filter(|r| !r.is_empty())?;
                Some(DocumentSelection {
                    course: Some(course.clone()),
                    resource: resource.into(),
                    title: file["display_name"]
                        .as_str()
                        .or(file["filename"].as_str())
                        .unwrap_or("Document")
                        .into(),
                    start: 0,
                })
            }
            _ => None,
        }
    }
    fn prepare_document(&mut self) {
        if let Some(document) = self.active_document()
            && self.document_resource != document.resource
        {
            self.document_resource = document.resource;
            self.document_page = 1;
            self.document_zoom = 100;
            self.document_view = DocumentView::Pages;
            self.document_jump_reset = true;
            self.document_scroll.reset(1);
        }
    }
    fn route_requests(&self) -> Vec<Request> {
        let mut requests = if matches!(self.route, Route::Document { .. }) {
            vec![]
        } else {
            model::requests(&self.route)
        };
        if let Route::Course {
            id,
            tab: CourseTab::Home,
        } = &self.route
            && let Some(data) = self.get("course_info", json!({"course_id":id}))
            && let Some(request) = model::course_home_request(id, &data["course"])
        {
            requests.push(request);
        }
        if let Some(document) = self.active_document() {
            requests.push(match self.document_view {
                DocumentView::Pages => model::document_preview_request(
                    &document.resource,
                    self.document_page,
                    self.document_zoom,
                ),
                DocumentView::Text => Request::new(
                    "attachment_text",
                    json!({"resource":document.resource,"start":document.start}),
                ),
            });
        }
        requests
    }
    fn document_page_count(&self) -> Option<u32> {
        self.data
            .values()
            .find(|v| v["document"]["resource"] == self.document_resource)
            .and_then(|v| v["document"]["pages"].as_u64())
            .and_then(|n| u32::try_from(n).ok())
    }
    fn showing_modules(&self) -> bool {
        match &self.route {
            Route::Course {
                tab: CourseTab::Modules,
                ..
            } => true,
            Route::Course {
                id,
                tab: CourseTab::Home,
            } => self
                .get("course_info", json!({"course_id":id}))
                .is_some_and(|v| model::course_home_view(&v["course"]) == "modules"),
            _ => false,
        }
    }
    fn showing_document_pages(&self) -> bool {
        matches!(self.route, Route::Document { .. }) && self.document_view == DocumentView::Pages
    }
    /// Opens a reader in its own window. Canvas links, module files and
    /// attachment rows all use this so the list the user came from stays put.
    fn open_document_window(&mut self, document: DocumentSelection, cx: &mut Context<Self>) {
        self.error = None;
        let settings = self.settings.clone();
        let runtime = self.runtime.clone();
        let backend = self.backend.clone();
        let demo = self.demo;
        let title = format!("{} — Canvas Native", document.title);
        let bounds = Bounds::centered(None, size(px(1120.), px(900.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                app_id: Some(crate::window_icon::APP_ID.into()),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(520.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                if let Err(error) = crate::window_icon::set_window_icon() {
                    eprintln!("Window icon unavailable: {error}");
                }
                let view = cx.new(|cx| {
                    CanvasApp::new_document(settings, &runtime, backend, demo, document, window, cx)
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        if let Err(error) = opened {
            self.error = Some(format!("Could not open the document window: {error}"));
            cx.notify();
        }
    }
    /// Resolves a file identifier to its Canvas resource without navigating the
    /// current screen, then opens the reader window.
    fn open_file_window(&mut self, course: String, file_id: String, cx: &mut Context<Self>) {
        self.error = None;
        let args = json!({"file_id": file_id});
        if let Some(data) = self.get("file_info", args.clone()).cloned() {
            if let Some(document) = document_from_file(&course, &data["file"]) {
                self.open_document_window(document, cx);
            } else {
                self.error = Some("This file has no previewable content.".into());
                cx.notify();
            }
            return;
        }
        self.pending_document_file = Some((course, file_id));
        self.enqueue_read(Request::new("file_info", args), false);
        self.pump(cx);
        cx.notify();
    }
    /// A file with a readable resource opens in the full-height reader. The
    /// file screen would otherwise nest a scrolling page list inside a
    /// scrolling page, where one wheel movement moves both.
    fn open_file_route(&mut self, cx: &mut Context<Self>) {
        let target = match &self.route {
            Route::File { course, id } => {
                self.get("file_info", json!({"file_id": id}))
                    .and_then(|data| {
                        let file = &data["file"];
                        let resource = file["resource"]
                            .as_str()
                            .or(file["download_resource"].as_str())?;
                        if resource.is_empty() {
                            return None;
                        }
                        let title = file["display_name"]
                            .as_str()
                            .or(file["filename"].as_str())
                            .unwrap_or("Document");
                        Some((course.clone(), resource.to_owned(), title.to_owned()))
                    })
            }
            _ => None,
        };
        if let Some((course, resource, title)) = target {
            // Replace the file screen rather than pushing it: Back still
            // returns to the list or module that opened the file.
            self.route = Route::Document {
                course: Some(course),
                resource,
                title,
                start: 0,
            };
        }
        self.load_route(cx);
    }
    fn set_document_page(&mut self, page: u32, cx: &mut Context<Self>) {
        self.error = None;
        let pages = self.document_page_count().unwrap_or(u32::MAX);
        self.document_page = page.max(1).min(pages);
        self.document_jump_reset = true;
        // Go, Previous and Next scroll the list; loading follows the visible page.
        self.document_scroll.scroll_to(gpui::ListOffset {
            item_ix: self.document_page.saturating_sub(1) as usize,
            offset_in_item: px(0.),
        });
        self.ensure_document_pages(cx);
        cx.notify();
    }
    fn set_document_view(&mut self, view: DocumentView, cx: &mut Context<Self>) {
        self.document_view = view;
        self.error = None;
        self.load_route(cx);
        cx.notify();
    }
    fn set_document_zoom(&mut self, zoom: u32, cx: &mut Context<Self>) {
        self.error = None;
        self.document_zoom = zoom.clamp(model::DOCUMENT_MIN_ZOOM, model::DOCUMENT_MAX_ZOOM);
        // Rasters are keyed by zoom and width, so the visible pages are fetched
        // again at the new size. Rows at the old size are not worth rendering.
        self.prefetch_queue.retain(
            |operation| !matches!(operation, Operation::Tool { name, .. } if name == "document_preview"),
        );
        self.document_scroll.scroll_to(gpui::ListOffset {
            item_ix: self.document_page.saturating_sub(1) as usize,
            offset_in_item: px(0.),
        });
        self.ensure_document_pages(cx);
        cx.notify();
    }
    /// Chooses the zoom that makes one whole page fit the current viewport.
    /// The action is one-shot: scrolling and manual zoom still work normally.
    fn fit_document_page(&mut self, cx: &mut Context<Self>) {
        let viewport = self.document_scroll.viewport_bounds().size;
        let available_width = f32::from(viewport.width);
        let available_height = f32::from(viewport.height);
        if available_width < 200.0 || available_height < 200.0 {
            self.set_document_zoom(100, cx);
            return;
        }
        let aspect = self
            .data
            .values()
            .find(|value| value["document"]["resource"] == self.document_resource)
            .and_then(|value| {
                Some(
                    value["document"]["image_height"].as_f64()?
                        / value["document"]["image_width"].as_f64()?,
                )
            })
            .filter(|aspect| aspect.is_finite() && *aspect > 0.0)
            .map(|aspect| aspect as f32)
            .unwrap_or(std::f32::consts::SQRT_2);
        let column = (available_width - 12.0).max(240.0);
        let page_height_at_full_width = column * aspect;
        let zoom = ((available_height - 12.0) / page_height_at_full_width * 100.0).floor();
        let zoom = zoom.clamp(
            model::DOCUMENT_MIN_ZOOM as f32,
            model::DOCUMENT_MAX_ZOOM as f32,
        ) as u32;
        self.set_document_zoom(zoom, cx);
    }
    fn document_scrolled(&mut self, visible: std::ops::Range<usize>, cx: &mut Context<Self>) {
        if self.document_view != DocumentView::Pages || self.active_document().is_none() {
            return;
        }
        let page = u32::try_from(visible.start.saturating_add(1))
            .unwrap_or(u32::MAX)
            .min(self.document_page_count().unwrap_or(u32::MAX));
        if page != self.document_page {
            self.document_page = page;
            self.document_jump_reset = true;
            cx.notify();
        }
        self.ensure_document_pages(cx);
    }
    fn sync_document_pages(&mut self, cx: &mut Context<Self>) {
        let Some(pages) = self.document_page_count() else {
            return;
        };
        let count = usize::try_from(pages).unwrap_or(1).max(1);
        if self.document_scroll.item_count() != count {
            self.document_scroll.reset(count);
            // Reset returns to the top. Keep the reader on its current page
            // when a document is reopened or its page count becomes known.
            let page = self.document_page.clamp(1, pages).saturating_sub(1) as usize;
            self.document_scroll.scroll_to(gpui::ListOffset {
                item_ix: page,
                offset_in_item: px(0.),
            });
        }
        self.ensure_document_pages(cx);
    }
    fn ensure_document_pages(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.active_document() else {
            return;
        };
        let Some(pages) = self.document_page_count() else {
            return;
        };
        if self.document_view != DocumentView::Pages || pages == 0 {
            return;
        }
        let first = self.document_page.clamp(1, pages);
        let keep = model::document_page_range(first, pages);
        let raster = model::document_preview_request(&doc.resource, first, self.document_zoom).args
            ["raster_size"]
            .clone();
        // Drop queued page work that the reader has scrolled past. The pages can
        // be requested again if they become visible; without this a fast scroll
        // would render every page it passed on the way.
        let mut queued = VecDeque::new();
        for operation in std::mem::take(&mut self.prefetch_queue) {
            let keep_operation = match &operation {
                Operation::Tool { name, args } if name == "document_preview" => {
                    args["resource"].as_str() == Some(doc.resource.as_str())
                        && args["raster_size"] == raster
                        && args["page"]
                            .as_u64()
                            .and_then(|page| u32::try_from(page).ok())
                            .is_some_and(|page| keep.contains(&page))
                }
                _ => true,
            };
            if keep_operation {
                queued.push_back(operation);
            } else if let Some(key) = operation_key(&operation) {
                self.prefetch_attempted.remove(&key);
            }
        }
        self.prefetch_queue = queued;
        for page in keep {
            let request = model::document_preview_request(&doc.resource, page, self.document_zoom);
            let key = model::cache_key(&request.name, &request.args);
            if self.document_images.contains_key(&key) || self.failures.contains_key(&key) {
                continue;
            }
            if self.prefetch_pending.contains_key(&key)
                || self
                    .queue
                    .iter()
                    .any(|o| operation_key(o).as_ref() == Some(&key))
                || self.pending.as_ref().and_then(operation_key).as_ref() == Some(&key)
            {
                continue;
            }
            // A text-only result needs no raster. Everything else is retried
            // when its image was evicted, but only once per page and session.
            if self.data.contains_key(&key) {
                let kind = self.data[&key]["document"]["kind"].as_str();
                if !matches!(kind, Some("pdf" | "image")) {
                    continue;
                }
            } else if self.prefetch_attempted.contains(&key) {
                continue;
            }
            self.prefetch_attempted.insert(key);
            self.prefetch_queue.push_back(Operation::Tool {
                name: request.name,
                args: request.args,
            });
        }
        self.pump_prefetch(cx);
    }
    fn retry_document_page(&mut self, page: u32, cx: &mut Context<Self>) {
        let Some(doc) = self.active_document() else {
            return;
        };
        let request = model::document_preview_request(&doc.resource, page, self.document_zoom);
        let key = model::cache_key(&request.name, &request.args);
        self.prefetch_attempted.remove(&key);
        self.enqueue_read(request, true);
        self.pump(cx);
    }
    fn cache_document_image(&mut self, key: String, image: Arc<Image>) {
        self.document_image_order.retain(|k| k != &key);
        self.document_image_order.push_back(key.clone());
        self.document_images.insert(key, image);
        let protected = self
            .active_document()
            .map(|doc| {
                model::document_preview_request(
                    &doc.resource,
                    self.document_page,
                    self.document_zoom,
                )
            })
            .map(|r| model::cache_key(&r.name, &r.args));
        // A scrolling viewport can hold two or three pages; keep a small margin
        // of rendered neighbours without turning a long PDF into a memory leak.
        while self.document_images.len() > 12 {
            let Some(old) = self.document_image_order.pop_front() else {
                break;
            };
            if Some(&old) == protected.as_ref() {
                self.document_image_order.push_back(old);
                continue;
            }
            self.document_images.remove(&old);
        }
    }
    fn clear_document_cache(&mut self) {
        self.document_images.clear();
        self.document_image_order.clear();
        self.document_resource.clear();
        self.document_scroll.reset(0);
    }
    /// The preview metadata is cached but its decoded image was evicted. The
    /// raster can be reused, so the page only needs to be decoded again.
    fn document_image_missing(&self, key: &str) -> bool {
        self.data.get(key).is_some_and(|value| {
            matches!(value["document"]["kind"].as_str(), Some("pdf" | "image"))
        }) && !self.document_images.contains_key(key)
    }
    fn enqueue_read(&mut self, request: Request, force: bool) {
        let operation = Operation::Tool {
            name: request.name,
            args: request.args,
        };
        let key = operation_key(&operation);
        if force && let Some(key) = &key {
            self.data.remove(key);
            self.failures.remove(key);
        }
        let missing_preview = matches!(&operation,Operation::Tool{name,..} if name=="document_preview")
            && key
                .as_deref()
                .is_some_and(|key| self.document_image_missing(key));
        if (!force && !missing_preview && key.as_ref().is_some_and(|k| self.data.contains_key(k)))
            || self.pending.as_ref().and_then(operation_key) == key && self.pending.is_some()
            || self.queue.iter().any(|o| operation_key(o) == key)
        {
            return;
        }
        if let Some(key) = &key {
            self.failures.remove(key);
            self.prefetch_queue
                .retain(|o| operation_key(o).as_ref() != Some(key));
        }
        self.queue.push_back(operation);
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.active_document().is_some() && self.pending.is_none() {
            self.backend.cancel();
            self.reset_prefetch();
            self.perform(
                Operation::Tool {
                    name: "document_reload".into(),
                    args: json!({}),
                },
                cx,
            );
            return;
        }
        let mut requests = self.route_requests();
        if matches!(self.route, Route::Dashboard | Route::Calendar) {
            requests.push(self.planner_request(self.route == Route::Dashboard));
        }
        for request in requests {
            self.enqueue_read(request, true);
        }
        self.pump(cx);
    }
    fn perform(&mut self, operation: Operation, cx: &mut Context<Self>) {
        self.queue.push_front(operation);
        self.pump(cx);
    }
    fn pump(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let Some(operation) = self.queue.pop_front() else {
            self.pump_prefetch(cx);
            return;
        };
        self.status = operation_label(&operation).into();
        self.pending = Some(operation.clone());
        if let Some(id) =
            operation_key(&operation).and_then(|key| self.prefetch_pending.get(&key).copied())
        {
            self.pending_id = Some(id);
            cx.notify();
            return;
        }
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.pending_id = Some(self.next_request_id);
        self.start_request(operation, self.next_request_id, cx);
    }
    fn queue_course_prefetch(&mut self) {
        for request in model::course_prefetch(&self.route) {
            let operation = Operation::Tool {
                name: request.name,
                args: request.args,
            };
            let key = operation_key(&operation).expect("tool key");
            if self.data.contains_key(&key)
                || self.prefetch_attempted.contains(&key)
                || self.prefetch_pending.contains_key(&key)
                || self.pending.as_ref().and_then(operation_key).as_ref() == Some(&key)
                || self
                    .queue
                    .iter()
                    .any(|o| operation_key(o).as_ref() == Some(&key))
            {
                continue;
            }
            self.prefetch_attempted.insert(key);
            self.prefetch_queue.push_back(operation);
        }
    }
    fn pump_prefetch(&mut self, cx: &mut Context<Self>) {
        if self.pending.as_ref().is_some_and(
            |o| !matches!(o,Operation::Tool{name,..}if crate::backend::is_cacheable_read(name)),
        ) {
            return;
        }
        while self.prefetch_pending.len() < 2 {
            let Some(operation) = self.prefetch_queue.pop_front() else {
                break;
            };
            let key = operation_key(&operation).expect("tool key");
            // A cached metadata entry normally means the request is done. A
            // document page whose decoded image was evicted is the exception:
            // it must be fetched again when it scrolls back into view.
            let replaces_evicted_image = matches!(
                &operation,
                Operation::Tool { name, .. } if name == "document_preview"
            ) && self.document_image_missing(&key);
            if (self.data.contains_key(&key) && !replaces_evicted_image)
                || self.prefetch_pending.contains_key(&key)
                || self.pending.as_ref().and_then(operation_key).as_ref() == Some(&key)
            {
                continue;
            }
            self.next_request_id = self.next_request_id.wrapping_add(1);
            let id = self.next_request_id;
            self.prefetch_pending.insert(key, id);
            self.start_request(operation, id, cx);
        }
    }
    fn reset_prefetch(&mut self) {
        self.session_generation = self.session_generation.wrapping_add(1);
        self.prefetch_pending.clear();
        self.prefetch_queue.clear();
        self.prefetch_attempted.clear();
    }
    fn start_request(&mut self, operation: Operation, id: u64, cx: &mut Context<Self>) {
        let generation = self.session_generation;
        let receiver = if self.demo && !matches!(operation, Operation::Appearance(_)) {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(crate::demo::respond(&operation));
            rx
        } else {
            self.backend.request(operation.clone())
        };
        cx.spawn(async move|this,cx|{
            let mut response=receiver.await.unwrap_or_else(|_|Err(anyhow::anyhow!("The Canvas connection stopped. Please restart the app.")));
            let mut prepared_image=None;
            if let (Operation::Tool{name,..},Ok(value))=(&operation,&response)&&name=="document_preview"&&value["isError"]!=true{
                let meta=&value["structuredContent"]["document"];
                if let Some(format)=meta["image_mime_type"].as_str().and_then(ImageFormat::from_mime_type){
                    let path=meta["image_path"].as_str().map(PathBuf::from);let encoded=meta["image_data"].as_str().map(str::to_owned);
                    match cx.background_executor().spawn(async move{
                        let bytes=if let Some(path)=path{std::fs::read(path)?}else{STANDARD.decode(encoded.ok_or_else(||anyhow::anyhow!("Missing preview image"))?)?};
                        if format==ImageFormat::Svg{crate::document_preview::validate_svg(&bytes)?;}
                        Ok::<_,anyhow::Error>(Arc::new(Image::from_bytes(format,bytes)))
                    }).await{
                        Ok(image)=>prepared_image=Some(image),Err(error)=>response=Err(anyhow::anyhow!("Could not read the rendered document page: {error}")),
                    }
                }
            }
            let mut prepared_module=if let (Operation::Tool{name,args},Ok(value))=(&operation,&response)
                && name=="module_list" && value["isError"]!=true && let Some(course)=args["course_id"].as_str(){
                let course=course.to_owned();let payload=schema::payload(value);
                Some(cx.background_executor().spawn(async move{let index=ModuleIndex::new(&course,&payload);(course,index)}).await)
            }else{None};
            let _=this.update(cx,|this,cx|{
                let visible=this.pending_id==Some(id);
                if visible{this.pending=None;this.pending_id=None;}
                if let Some(key)=operation_key(&operation)&&this.prefetch_pending.get(&key)==Some(&id){this.prefetch_pending.remove(&key);}
                if generation!=this.session_generation{this.pump(cx);cx.notify();return;}
                match response{
                    Ok(value) if value["isError"]!=true=>{
                        let payload=schema::payload(&value);
                        if let Some(key)=operation_key(&operation){this.failures.remove(&key);this.data.insert(key,payload.clone());}
                        if visible{this.updated=Some(Local::now().format("%-I:%M %p").to_string());this.status=if this.demo{"Demo · sample data".into()}else{"Up to date".into()};}
                        match &operation{
                            Operation::Tool{name,args} if name=="course_info"=>{
                                if matches!(&this.route, Route::Course{id,tab:CourseTab::Home} if args["course_id"]==*id) {
                                    this.rebuild_module_rows(cx);this.load_route(cx);
                                }
                            },
                            Operation::Configure(settings)=>{
                                this.reset_prefetch();
                                this.history.clear();
                                let appearance=this.settings.appearance;this.settings=settings.clone();this.settings.appearance=appearance;
                                this.data.clear();this.clear_module_cache();this.clear_document_cache();this.resource_images.clear();this.failures.clear();this.queue.clear();
                                if settings.appearance!=appearance{this.queue.push_back(Operation::Appearance(appearance));}
                                this.queue.push_back(Operation::Tool{name:"refresh_auth".into(),args:json!({})});
                            },
                            Operation::Tool{name,..} if matches!(name.as_str(),"auth"|"refresh_auth")=>{
                                this.reset_prefetch();
                                this.history.clear();
                                this.route=Route::Dashboard;this.data.clear();this.clear_module_cache();this.clear_document_cache();this.resource_images.clear();this.load_route(cx);
                            },
                            Operation::Tool{name,..} if schema::is_write(name)=>{
                                this.data.retain(|k,_|!k.starts_with("dashboard_items:")&&!k.starts_with("planner_note_info:"));
                                this.note=None;this.confirm_save=false;this.confirm_delete=None;this.route=Route::Calendar;this.load_route(cx);
                            },
                            Operation::Tool{name,..} if name=="download_attachment"=>{this.last_saved=payload["saved_path"].as_str().map(PathBuf::from);},
                            Operation::Tool{name,..} if name=="module_list"=>{
                                if let Some((course,index))=prepared_module.take(){this.module_indexes.insert(course,index);this.rebuild_module_rows(cx);}
                            },
                            Operation::Tool{name,args} if name=="document_preview"=>{
                                if let Some(image)=prepared_image.take()&&let Some(key)=operation_key(&operation){this.cache_document_image(key,image);}
                                if this.active_document().is_some_and(|doc|args["resource"]==doc.resource){
                                    if payload["document"]["kind"]=="text"{
                                        if visible&&this.document_view==DocumentView::Pages{this.document_view=DocumentView::Text;this.load_route(cx);}
                                    }else{this.sync_document_pages(cx);}
                                }
                            },
                            Operation::Tool{name,args} if name=="file_info"=>{
                                let requested=args["file_id"].as_str().unwrap_or("");
                                if let Some((course,_))=this.pending_document_file.clone().filter(|(_,id)|id==requested){
                                    this.pending_document_file=None;
                                    if let Some(document)=document_from_file(&course,&payload["file"]){
                                        this.open_document_window(document,cx);
                                    }else{
                                        this.error=Some("This file has no previewable content.".into());
                                        cx.notify();
                                    }
                                }else if visible{this.open_file_route(cx);}
                            },
                            Operation::Tool{name,..} if name=="document_reload"=>{
                                this.data.retain(|key,_|!key.starts_with("document_preview:")&&!key.starts_with("attachment_text:")&&!key.starts_with("attachment_image:")&&!key.starts_with("resource:"));
                                this.clear_document_cache();this.resource_images.clear();this.image=None;this.image_content=None;
                                if let Route::File{id,..}=&this.route{this.data.remove(&model::cache_key("file_info",&json!({"file_id":id})));}
                                this.load_route(cx);
                            },
                            Operation::Tool{name,args} if name=="attachment_image"=>{
                                if matches!(&this.route,Route::Document{resource,..} if args["resource"]==*resource){this.image=decode_image(&value);this.image_content=Some(value);}
                            },
                            Operation::Resource(uri)=>{
                                this.resource_images.remove(uri);
                                if let Some(image)=decode_image(&value){this.resource_images.insert(uri.clone(),image);}
                            },_=>{},
                        }
                        if visible&&matches!(&operation,Operation::Tool{name,..}if crate::backend::is_cacheable_read(name)){this.queue_course_prefetch();}
                    },
                    result=>{
                        let error=match result{Ok(value)=>model::display(&schema::payload(&value)),Err(error)=>format!("{error:#}")};
                        if let Some(key)=operation_key(&operation){this.failures.insert(key,error.clone());}
                        if visible{this.error=Some(error);this.status="Could not update Canvas".into();}
                    }
                }
                this.pump(cx);this.pump_prefetch(cx);cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn open_note(
        &mut self,
        value: Option<&Value>,
        linked: Option<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let today = Local::now().date_naive().to_string();
        let date = value
            .map(|v| field(v, "todo_date"))
            .filter(|s| !s.is_empty())
            .unwrap_or(today);
        let time = chrono::DateTime::parse_from_rfc3339(&date)
            .ok()
            .map(|time| time.with_timezone(&Local).format("%H:%M").to_string())
            .unwrap_or_default();
        let selected_course = value
            .and_then(|v| v["course_id"].as_str())
            .map(str::to_owned)
            .or_else(|| {
                linked
                    .as_ref()
                    .and_then(|v| v["course"].as_str())
                    .map(str::to_owned)
            })
            .or_else(|| self.route.course_id().map(str::to_owned));
        self.note = Some(NoteEditor {
            id: value.and_then(|v| v["id"].as_str()).map(str::to_owned),
            title: input(
                &value.map(|v| field(v, "title")).unwrap_or_default(),
                "What do you need to do?",
                false,
                window,
                cx,
            ),
            details: input(
                &value.map(|v| field(v, "details")).unwrap_or_default(),
                "Add details (optional)",
                true,
                window,
                cx,
            ),
            date: input(
                &model::date(&date).map(|d| d.to_string()).unwrap_or(date),
                "YYYY-MM-DD",
                false,
                window,
                cx,
            ),
            time: input(&time, "HH:MM (optional)", false, window, cx),
            course: selected_course,
            linked,
        });
        self.confirm_save = false;
        self.picker = None;
        cx.notify();
    }
    fn set_appearance(
        &mut self,
        appearance: crate::appearance::Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.appearance = appearance;
        appearance.apply(Some(window), cx);
        self.queue
            .retain(|operation| !matches!(operation, Operation::Appearance(_)));
        self.perform(Operation::Appearance(appearance), cx);
        cx.notify();
    }
    fn save_note(&mut self, cx: &mut Context<Self>) {
        let Some(note) = &self.note else {
            return;
        };
        let title = note.title.read(cx).value().to_string();
        let date = note.date.read(cx).value().to_string();
        if title.trim().is_empty() {
            self.error = Some("Give your to-do a title.".into());
            cx.notify();
            return;
        }
        let date = match model::scheduled_at(&date, &note.time.read(cx).value()) {
            Ok(date) => date,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        if !self.confirm_save {
            self.confirm_save = true;
            cx.notify();
            return;
        }
        let mut args = json!({"title":title,"details":note.details.read(cx).value().to_string(),"todo_date":date,"course_id":note.course.clone().unwrap_or_default()});
        let name = if let Some(id) = &note.id {
            args["note_id"] = json!(id);
            "update_planner_note"
        } else {
            if note.course.is_none() {
                args.as_object_mut().unwrap().remove("course_id");
            }
            if let Some(linked) = &note.linked {
                args["linked_object_type"] = linked["kind"].clone();
                args["linked_object_id"] = linked["id"].clone();
            }
            "create_planner_note"
        };
        self.perform(
            Operation::Tool {
                name: name.into(),
                args,
            },
            cx,
        );
    }
    fn download(&mut self, file: &Value, save_as: bool, cx: &mut Context<Self>) {
        let resource = field(file, "download_resource");
        let resource = if resource.is_empty() {
            field(file, "resource")
        } else {
            resource
        };
        if resource.is_empty() {
            return;
        }
        let name = field(file, "filename");
        if !save_as {
            self.perform(
                Operation::Tool {
                    name: "download_attachment".into(),
                    args: json!({"resource":resource}),
                },
                cx,
            );
            return;
        }
        let directory = directories::UserDirs::new()
            .map(|d| d.download_dir().unwrap_or(d.home_dir()).to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let picker = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let chosen = picker.await;
            let _ = this.update(cx, |this, cx| match chosen {
                Ok(Ok(Some(path))) => this.perform(
                    Operation::Tool {
                        name: "download_attachment".into(),
                        args: json!({"resource":resource,"destination_path":path}),
                    },
                    cx,
                ),
                Ok(Ok(None)) => {}
                _ => {
                    this.error = Some("The Save As dialog could not be opened.".into());
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn save_image(&mut self, cx: &mut Context<Self>) {
        let Some(content) = self
            .image_content
            .as_ref()
            .and_then(|v| v["content"].as_array())
            .and_then(|a| a.iter().find(|c| c["type"] == "image"))
        else {
            return;
        };
        let Some(encoded) = content["data"].as_str().map(str::to_owned) else {
            return;
        };
        let extension = match content["mimeType"].as_str() {
            Some("image/jpeg") => "jpg",
            Some("image/jp2") => "jp2",
            _ => "png",
        };
        let directory = directories::UserDirs::new()
            .map(|d| d.download_dir().unwrap_or(d.home_dir()).to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let picker =
            cx.prompt_for_new_path(&directory, Some(&format!("document-image.{extension}")));
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<Option<PathBuf>> = async {
                let Some(path) = picker.await?? else {
                    return Ok(None);
                };
                cx.background_executor()
                    .spawn(async move {
                        std::fs::write(&path, STANDARD.decode(encoded)?)?;
                        Ok(Some(path))
                    })
                    .await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Some(path)) => this.last_saved = Some(path),
                    Ok(None) => {}
                    Err(e) => this.error = Some(format!("Could not save image: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
fn operation_key(operation: &Operation) -> Option<String> {
    match operation {
        Operation::Tool { name, args } => Some(model::cache_key(name, args)),
        Operation::Resource(uri) => Some(format!("resource:{uri}")),
        _ => None,
    }
}
fn operation_label(operation: &Operation) -> &'static str {
    match operation {
        Operation::Appearance(_) => "Saving appearance…",
        Operation::Configure(_) => "Connecting to your university…",
        Operation::Discover => "Checking available features…",
        Operation::Resource(_) => "Opening document…",
        Operation::Tool { name, .. } => match name.as_str() {
            "course_list" => "Loading your courses…",
            "dashboard_items" => "Loading your to-do list…",
            "auth" => "Sign in in the browser, then close its window to continue.",
            "refresh_auth" => "Restoring your saved login…",
            "download_attachment" => "Downloading file…",
            name if schema::is_write(name) => "Saving your to-do…",
            _ => "Loading course content…",
        },
    }
}
fn document_from_file(course: &str, file: &Value) -> Option<DocumentSelection> {
    let resource = file["resource"]
        .as_str()
        .or(file["download_resource"].as_str())?;
    if resource.is_empty() {
        return None;
    }
    let title = file["display_name"]
        .as_str()
        .or(file["filename"].as_str())
        .unwrap_or("Document");
    Some(DocumentSelection {
        course: Some(course.into()),
        resource: resource.into(),
        title: title.into(),
        start: 0,
    })
}
fn decode_image(result: &Value) -> Option<Arc<Image>> {
    let block = result["content"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["type"] == "image"))
        .or_else(|| {
            result["contents"]
                .as_array()
                .and_then(|a| a.iter().find(|c| c.get("blob").is_some()))
        })?;
    let format = ImageFormat::from_mime_type(block["mimeType"].as_str()?)?;
    let bytes = STANDARD
        .decode(block["data"].as_str().or(block["blob"].as_str())?)
        .ok()?;
    if format == ImageFormat::Svg {
        crate::document_preview::validate_svg(&bytes).ok()?;
    }
    Some(Arc::new(Image::from_bytes(format, bytes)))
}

#[cfg(test)]
mod reader_layout_tests {
    use super::*;
    use gpui::{ScrollDelta, ScrollWheelEvent, TestAppContext, point, px};

    #[test]
    fn file_metadata_carries_the_canvas_resource_into_the_reader() {
        let file = json!({
            "resource": "canvas://files/7/download",
            "display_name": "Reading.pdf",
            "filename": "Reading.pdf"
        });
        let document = document_from_file("4", &file).expect("previewable file");
        assert_eq!(document.course.as_deref(), Some("4"));
        assert_eq!(document.resource, "canvas://files/7/download");
        assert_eq!(document.title, "Reading.pdf");
        let fallback = json!({"download_resource": "canvas://files/8/download"});
        assert_eq!(
            document_from_file("4", &fallback)
                .expect("download resource fallback")
                .resource,
            "canvas://files/8/download"
        );
        assert!(document_from_file("4", &json!({"resource": ""})).is_none());
    }

    /// Scrolling through more than the decoded-image cache evicts early pages.
    /// Their preview metadata stays cached, so the prefetch pump must still
    /// request the raster again when those pages scroll back into view.
    #[gpui::test]
    fn evicted_document_page_is_requested_again(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("test runtime");
        let handle = runtime.handle().clone();
        let mut built = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            gpui_component::init(cx);
            let app = cx.new(|cx| CanvasApp::new(Settings::default(), &handle, false, window, cx));
            built = Some(app.clone());
            gpui_component::Root::new(app, window, cx)
        });
        let app = built.expect("app entity");
        let resource = "canvas://files/9/download";
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.route = Route::Document {
                    course: None,
                    resource: resource.into(),
                    title: "Reading.pdf".into(),
                    start: 0,
                };
                app.document_resource = resource.into();
                app.document_page = 1;
                app.document_zoom = 100;
                app.document_view = DocumentView::Pages;
                let request = model::document_preview_request(resource, 1, 100);
                let key = model::cache_key(&request.name, &request.args);
                app.data.insert(
                    key.clone(),
                    json!({"document": {
                        "resource": resource,
                        "kind": "pdf",
                        "pages": 6,
                        "page": 1,
                        "image_width": 1273,
                        "image_height": 1800,
                    }}),
                );
                assert!(app.document_images.is_empty());
                app.prefetch_queue.push_back(Operation::Tool {
                    name: request.name,
                    args: request.args,
                });
                app.pump_prefetch(cx);
                assert!(
                    app.prefetch_pending.contains_key(&key),
                    "a page whose decoded image was evicted must be fetched again"
                );
            });
        });
    }

    fn page_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 64, 90);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            writer
                .write_image_data(&vec![0xEE; 64 * 90 * 4])
                .expect("png data");
        }
        bytes
    }

    /// Opens a real GPUI window on the headless test platform, gives it a
    /// document route with six rendered pages, and scrolls the page list.
    ///
    /// This exercises the list's item renderer, which runs while GPUI holds a
    /// mutable borrow of the list state. Reading that state from a row panicked
    /// at runtime with "RefCell already mutably borrowed".
    #[gpui::test]
    fn scrolled_pages_render_without_reborrowing_the_list(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("test runtime");
        let handle = runtime.handle().clone();
        let mut built = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            gpui_component::init(cx);
            let app = cx.new(|cx| CanvasApp::new(Settings::default(), &handle, false, window, cx));
            built = Some(app.clone());
            gpui_component::Root::new(app, window, cx)
        });
        let app = built.expect("app entity");
        let resource = "canvas://files/7/download";
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.route = Route::Document {
                    course: None,
                    resource: resource.into(),
                    title: "Reading.pdf".into(),
                    start: 0,
                };
                app.document_resource = resource.into();
                app.document_page = 3;
                app.document_zoom = 100;
                app.document_view = DocumentView::Pages;
                let image = Arc::new(Image::from_bytes(ImageFormat::Png, page_png()));
                for page in 1..=6 {
                    let request = model::document_preview_request(resource, page, 100);
                    let key = model::cache_key(&request.name, &request.args);
                    app.data.insert(
                        key.clone(),
                        json!({"document": {
                            "resource": resource,
                            "filename": "Reading.pdf",
                            "mime_type": "application/pdf",
                            "kind": "pdf",
                            "pages": 6,
                            "page": page,
                            "raster_size": 1800,
                            "image_width": 1273,
                            "image_height": 1800,
                        }}),
                    );
                    app.cache_document_image(key, image.clone());
                }
                app.sync_document_pages(cx);
                assert_eq!(app.document_scroll.item_count(), 6);
            });
            window.draw(cx).clear();
        });
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(700.), px(400.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear();
        });
        let top = cx.update(|_, cx| app.read(cx).document_scroll.logical_scroll_top().item_ix);
        assert!(top > 0, "page list should scroll, top item is {top}");
        // A zoom change replaces every page key, so the next frame lays out
        // placeholder rows until the new rasters arrive.
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.set_document_zoom(150, cx));
            window.draw(cx).clear();
        });
        assert_eq!(cx.update(|_, cx| app.read(cx).document_zoom), 150);
        // The same viewer embedded in a file screen uses a fixed list height.
        // That path used to collapse to zero height because flex_1() left a
        // zero flex-basis that flex_none() did not reset.
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let file_key = model::cache_key("file_info", &json!({"file_id": "7"}));
                app.data.insert(
                    file_key,
                    json!({"file": {
                        "id": "7",
                        "display_name": "Reading.pdf",
                        "filename": "Reading.pdf",
                        "resource": resource,
                        "download_resource": resource,
                        "size": 1234,
                        "mime_class": "pdf",
                    }}),
                );
                app.route = Route::File {
                    course: "1".into(),
                    id: "7".into(),
                };
                app.document_page = 1;
                app.document_zoom = 100;
                app.load_route(cx);
            });
            window.draw(cx).clear();
        });
        let height = cx
            .update(|_, cx| f32::from(app.read(cx).document_scroll.viewport_bounds().size.height));
        assert!(height > 100.0, "embedded reader list height is {height}");
        // A file with a readable resource replaces its metadata screen with the
        // full-height reader, so there is only one vertical scroll container.
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.route = Route::File {
                    course: "1".into(),
                    id: "7".into(),
                };
                app.open_file_route(cx);
                assert!(matches!(
                    &app.route,
                    Route::Document { resource, .. } if resource == "canvas://files/7/download"
                ));
            });
        });
    }
}
