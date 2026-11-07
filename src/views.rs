use super::*;
use crate::appearance::{Appearance, palette};
use gpui::{
    Animation, AnimationExt, AnyElement, Div, FontWeight, ObjectFit, SharedString, Transformation,
    div, img, percentage, px, rgb,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::Input,
    skeleton::Skeleton,
    text::TextView,
};

fn muted(text: impl Into<SharedString>, cx: &gpui::App) -> Div {
    let colors = palette(cx);
    div()
        .text_sm()
        .text_color(rgb(colors.muted))
        .child(text.into())
}
fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .text_2xl()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}
fn box_view(cx: &gpui::App) -> Div {
    let colors = palette(cx);
    div()
        .flex()
        .flex_col()
        .gap_4()
        .p_5()
        .border_1()
        .border_color(rgb(colors.border))
        .rounded_md()
        .bg(rgb(colors.surface))
}
/// A single-arc spinner that turns at a constant speed. The stock sunburst
/// icon has eight-fold symmetry and the default eased spinner visibly stalls
/// between turns; the arc reads cleanly on light and dark surfaces.
fn loading_spinner(cx: &gpui::App) -> AnyElement {
    let colors = palette(cx);
    Icon::new(IconName::LoaderCircle)
        .small()
        .text_color(rgb(colors.muted))
        .with_animation(
            "canvas-loading-spinner",
            Animation::new(std::time::Duration::from_secs(1)).repeat(),
            |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
        )
        .into_any_element()
}
fn rule(cx: &gpui::App) -> Div {
    let colors = palette(cx);
    div().w_full().h(px(1.)).bg(rgb(colors.border))
}
/// A page row scrolls sideways only when it is zoomed past the reader width.
/// Restricting the scroll axis keeps vertical wheel deltas with the page list
/// instead of turning them into horizontal movement inside one page.
fn page_side_scroll<T: gpui::Styled>(mut div: T) -> T {
    div.style().restrict_scroll_to_axis = Some(true);
    div
}
fn badge(text: &str, cx: &gpui::App) -> Div {
    let colors = palette(cx);
    div()
        .px_2()
        .py_1()
        .rounded_sm()
        .text_xs()
        .bg(rgb(match text {
            "Missing" | "Late" => colors.missing,
            "Submitted" | "Graded" | "Completed" => colors.complete,
            _ => colors.subtle,
        }))
        .text_color(rgb(colors.text))
        .child(text.to_owned())
}
fn icon_slot(icon: IconName, color: impl Into<gpui::Hsla>) -> Div {
    div()
        .size(px(24.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(icon).size(px(18.)).text_color(color.into()))
}
// List items are measured as independent layout roots, not stretched flex
// children. A flex row with auto width therefore shrinks to its text contents.
fn module_row_container() -> Div {
    div().w_full().min_w_0()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReaderLayout {
    /// A dedicated reader window: controls plus pages, nothing else.
    Window,
    /// The document route inside the main window.
    Route,
    /// A reader embedded in a file screen.
    Embedded,
}
impl CanvasApp {
    fn route_button(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        route: Route,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id.into())
            .label(label.to_owned())
            .small()
            .on_click(cx.listener(move |this, _, window, cx| this.go(route.clone(), window, cx)))
    }
    fn route_link(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        route: Route,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        div()
            .id(id.into())
            .cursor_pointer()
            .text_color(rgb(colors.link))
            .hover(|s| s.underline())
            .child(label.to_owned())
            .on_click(cx.listener(move |this, _, window, cx| this.go(route.clone(), window, cx)))
            .into_any_element()
    }
    /// A document link opens a reader window and leaves the current list in
    /// place, matching the way a PDF opens from a file browser.
    fn document_link(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        document: DocumentSelection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        div()
            .id(id.into())
            .cursor_pointer()
            .text_color(rgb(colors.link))
            .hover(|s| s.underline())
            .child(label.to_owned())
            .on_click(
                cx.listener(move |this, _, _, cx| this.open_document_window(document.clone(), cx)),
            )
            .into_any_element()
    }
    /// Module file rows only carry an identifier. Resolve it, then open the
    /// reader window rather than navigating the main window away from Modules.
    fn file_document_link(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        course: &str,
        file_id: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        let course = course.to_owned();
        div()
            .id(id.into())
            .cursor_pointer()
            .text_color(rgb(colors.link))
            .hover(|s| s.underline())
            .child(label.to_owned())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_file_window(course.clone(), file_id.clone(), cx)
            }))
            .into_any_element()
    }
    /// Canvas HTML uses ordinary anchors, which a text view opens in the system
    /// browser. Canvas file anchors are moved out of the body and rendered as
    /// native reader actions so every file goes to the in-app viewer.
    fn canvas_body(
        &self,
        id: impl Into<SharedString>,
        html: &str,
        course: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (body, links) = crate::html_content::prepare_canvas_page(html, course);
        let body_text = crate::html_content::notice_text(&body);
        let only_link_body =
            links.len() == 1 && body_text.trim().eq_ignore_ascii_case(links[0].title.trim());
        let mut view = div().w_full().min_w_0().flex().flex_col().gap_4();
        if !links.is_empty() {
            view = view.child(self.canvas_file_links(&links, cx));
        }
        if !only_link_body {
            view = view.child(
                TextView::html(id.into(), SharedString::from(body), window, cx).selectable(true),
            );
        }
        view.into_any_element()
    }
    fn canvas_file_links(
        &self,
        links: &[crate::html_content::CanvasFileLink],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        let mut files =
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .border_1()
                .border_color(rgb(colors.border))
                .rounded_md()
                .bg(rgb(colors.notice))
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                    if links.len() == 1 {
                        "File in this page"
                    } else {
                        "Files in this page"
                    },
                ));
        for link in links {
            let course = link.course.clone();
            let file_id = link.file_id.clone();
            files = files.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon_slot(IconName::File, rgb(colors.muted)))
                    .child(self.file_document_link(
                        format!("page-file-{}-{}", link.course, link.file_id),
                        &link.title,
                        &course,
                        file_id,
                        cx,
                    )),
            );
        }
        files.into_any_element()
    }
    fn web_link(&self, id: impl Into<SharedString>, label: &str, url: &str) -> AnyElement {
        let url = url.to_owned();
        let safe = url::Url::parse(&url).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.username().is_empty()
                && u.password().is_none()
        });
        Button::new(id.into())
            .label(label.to_owned())
            .small()
            .disabled(!safe)
            .on_click(move |_, _, cx| cx.open_url(&url))
            .into_any_element()
    }
    fn web_text_link(
        &self,
        id: impl Into<SharedString>,
        label: &str,
        url: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        let url = url.to_owned();
        div()
            .id(id.into())
            .cursor_pointer()
            .text_color(rgb(colors.link))
            .hover(|s| s.underline())
            .child(label.to_owned())
            .on_click(move |_, _, cx| cx.open_url(&url))
            .into_any_element()
    }
    fn global_nav(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mut nav = div()
            .flex()
            .flex_col()
            .w(px(88.))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(colors.nav))
            .text_color(rgb(0xffffff));
        for (label, icon, route) in [
            ("Account", IconName::CircleUser, Route::Account),
            ("Dashboard", IconName::LayoutDashboard, Route::Dashboard),
            ("Courses", IconName::BookOpen, Route::Courses),
            ("Calendar", IconName::Calendar, Route::Calendar),
            ("Grades", IconName::ChartPie, Route::Grades),
        ] {
            let active = self.route == route
                || (route == Route::Courses && self.courses_open)
                || (route == Route::Courses && self.route.course_id().is_some());
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("global-{label}")))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .h(px(82.))
                    .cursor_pointer()
                    .bg(rgb(if active {
                        colors.background
                    } else {
                        colors.nav
                    }))
                    .text_color(rgb(if active { colors.link } else { colors.nav_text }))
                    .hover(|s| {
                        s.bg(rgb(if active {
                            colors.hover
                        } else {
                            colors.nav_hover
                        }))
                    })
                    .child(Icon::new(icon).size(px(26.)))
                    .child(div().text_xs().child(label))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if route == Route::Courses {
                            if this.courses_open {
                                this.close_courses(window, cx);
                                return;
                            }
                            this.courses_previous_focus = window.focused(cx);
                            this.courses_open = true;
                            window.focus(&this.courses_focus);
                            this.enqueue_read(Request::new("course_list", json!({})), false);
                            this.pump(cx);
                            cx.notify();
                        } else {
                            this.go(route.clone(), window, cx);
                        }
                    })),
            );
        }
        nav.child(div().flex_1())
            .child(
                Button::new("toggle-appearance")
                    .icon(if cx.theme().mode.is_dark() {
                        IconName::Sun
                    } else {
                        IconName::Moon
                    })
                    .label("Theme")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| {
                        let appearance = if cx.theme().mode.is_dark() {
                            Appearance::Light
                        } else {
                            Appearance::Dark
                        };
                        this.set_appearance(appearance, window, cx);
                    })),
            )
            .child(
                div()
                    .px_2()
                    .py_4()
                    .text_xs()
                    .text_center()
                    .child(if self.demo {
                        "SAMPLE DATA"
                    } else {
                        "Canvas Native"
                    }),
            )
            .into_any_element()
    }
    fn course_nav(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mut nav = div()
            .w(px(188.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .px_5()
            .py_6()
            .child(
                muted(
                    self.courses()
                        .into_iter()
                        .find(|v| field(v, "id") == id)
                        .map(|v| field(&v["term"], "name"))
                        .unwrap_or_default(),
                    cx,
                )
                .mb_4(),
            );
        for tab in [
            CourseTab::Home,
            CourseTab::Modules,
            CourseTab::Assignments,
            CourseTab::Grades,
            CourseTab::Files,
            CourseTab::Syllabus,
        ] {
            let active = matches!(&self.route,Route::Course{tab:t,..}if *t==tab)
                || matches!(tab, CourseTab::Assignments)
                    && matches!(self.route, Route::Assignment { .. });
            let route = Route::Course { id: id.into(), tab };
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("course-nav-{}", tab.title())))
                    .py_3()
                    .px_3()
                    .border_l_2()
                    .border_color(rgb(if active {
                        colors.text
                    } else {
                        colors.background
                    }))
                    .text_color(rgb(if active { colors.text } else { colors.link }))
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child(tab.title())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.go(route.clone(), window, cx)),
                    ),
            );
        }
        nav.into_any_element()
    }
    fn courses_drawer(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let panel = div()
            .id("courses-drawer")
            .occlude()
            .track_focus(&self.courses_focus)
            .key_context("CoursesDrawer")
            .on_action(cx.listener(|this, _: &crate::CloseCourses, window, cx| {
                this.close_courses(window, cx)
            }))
            .absolute()
            .left(px(88.))
            .top_0()
            .bottom_0()
            .w(px(390.))
            .bg(rgb(colors.background))
            .border_r_1()
            .border_color(rgb(colors.border))
            .shadow_lg()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_5()
                    .child(heading("Courses"))
                    .child(
                        Button::new("close-courses")
                            .icon(IconName::Close)
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_courses(window, cx)),
                            ),
                    ),
            )
            .child(div().px_5().pb_5().child(self.route_link(
                "drawer-all-courses",
                "All Courses",
                Route::Courses,
                cx,
            )))
            .child(rule(cx));
        let mut list = div()
            .id("courses-drawer-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .flex()
            .flex_col()
            .gap_5();
        if self.get("course_list", json!({})).is_none() {
            list = list.child(self.loading("course_list", json!({}), cx));
        } else {
            list = list.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Published Courses"),
            );
            let mut courses: Vec<_> = self
                .courses()
                .into_iter()
                .filter(|v| model::course_is_current(v))
                .collect();
            courses.sort_by_key(|v| model::course_title(v).to_lowercase());
            for course in courses {
                let id = field(course, "id");
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(self.route_link(
                            format!("drawer-course-{id}"),
                            &model::course_title(course),
                            Route::Course {
                                id,
                                tab: CourseTab::Home,
                            },
                            cx,
                        ))
                        .child(muted(field(course, "course_code"), cx))
                        .child(muted(
                            format!("Term: {}", field(&course["term"], "name")),
                            cx,
                        )),
                );
            }
        }
        panel.child(list).into_any_element()
    }
    fn page_title(&self) -> String {
        match &self.route {
            Route::Dashboard => "Dashboard".into(),
            Route::Courses => "All Courses".into(),
            Route::Calendar => "Calendar".into(),
            Route::Grades => "Grades".into(),
            Route::Account => "Account".into(),
            Route::Course { tab, .. } => tab.title().into(),
            Route::Assignment { course, id } => self
                .get(
                    "assignment_info",
                    json!({"course_id":course,"assignment_id":id}),
                )
                .map(|v| field(&v["assignment"], "name"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Assignment".into()),
            Route::Page { course, slug } => self
                .get("page_info", json!({"course_id":course,"page_url":slug}))
                .map(|v| field(&v["page"], "title"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Page".into()),
            Route::File { id, .. } => self
                .get("file_info", json!({"file_id":id}))
                .map(|v| field(&v["file"], "display_name"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "File".into()),
            Route::Document { title, .. } => title.clone(),
            Route::Note(_) => "To-do".into(),
        }
    }
    fn loading(&self, name: &str, args: Value, cx: &mut Context<Self>) -> AnyElement {
        let key = model::cache_key(name, &args);
        if self.failures.contains_key(&key) {
            let name = name.to_owned();
            return box_view(cx).child(heading("Couldn’t load this content"))
                .child(muted("Check your connection, then try again. Your other courses are still available.", cx))
                .child(Button::new("retry-content").label("Try again").small().on_click(cx.listener(move|this,_,_,cx| {
                    this.enqueue_read(Request::new(&name, args.clone()), true);this.pump(cx);cx.notify();
                }))).into_any_element();
        }
        let label = match name {
            "course_list" => "Loading your courses",
            "module_list" => "Loading course modules",
            "assignment_info" => "Opening assignment",
            "assignment_list" => "Loading assignments",
            "course_info" | "course_front_page" => "Opening your course",
            "course_grades" | "grade_summary" => "Loading grades",
            "document_preview" => "Preparing your document",
            _ => "Loading content",
        };
        let mut view = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(loading_spinner(cx))
                    .child(label),
            )
            .child(muted("Getting the latest from Canvas…", cx));
        // The stock skeleton colour nearly disappears on dark surfaces; use a
        // surface-relative colour with visible contrast in both themes.
        let colors = palette(cx);
        let skeleton = if colors.dark {
            colors.border
        } else {
            colors.subtle
        };
        for index in 0..3usize {
            view = view.child(
                div()
                    .id(("loading-placeholder", index))
                    .w_full()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        Skeleton::new()
                            .w(gpui::relative(0.48))
                            .h(px(20.))
                            .rounded_sm()
                            .bg(rgb(skeleton)),
                    )
                    .child(
                        Skeleton::new()
                            .w(gpui::relative(0.85))
                            .rounded_sm()
                            .secondary()
                            .bg(rgb(skeleton)),
                    )
                    .child(
                        Skeleton::new()
                            .w(gpui::relative(0.65))
                            .rounded_sm()
                            .secondary()
                            .bg(rgb(skeleton)),
                    ),
            );
        }
        view.into_any_element()
    }
    fn dashboard(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let courses = self.courses();
        if self.get("course_list", json!({})).is_none() {
            return self.loading("course_list", json!({}), cx);
        }
        let mut cards = div().flex().flex_wrap().gap_6();
        let query = self.search.read(cx).value().to_lowercase();
        let visible: Vec<_> = courses
            .into_iter()
            .filter(|v| {
                model::course_is_current(v) && v.to_string().to_lowercase().contains(&query)
            })
            .collect();
        for course in &visible {
            let id = field(course, "id");
            let color = model::course_color(&id);
            let route = Route::Course {
                id: id.clone(),
                tab: CourseTab::Home,
            };
            let mut shortcuts = div()
                .flex()
                .justify_between()
                .gap_3()
                .pt_3()
                .border_t_1()
                .border_color(rgb(colors.border));
            for (label, icon, tab) in [
                ("Modules", IconName::BookOpen, CourseTab::Modules),
                ("Assignments", IconName::File, CourseTab::Assignments),
                ("Grades", IconName::ChartPie, CourseTab::Grades),
            ] {
                shortcuts = shortcuts.child(
                    div().flex_1().min_w_0().child(
                        Button::new(SharedString::from(format!("card-{id}-{label}")))
                            .icon(icon)
                            .label(label)
                            .small()
                            .w_full()
                            .on_click(cx.listener({
                                let id = id.clone();
                                move |this, _, window, cx| {
                                    this.go(
                                        Route::Course {
                                            id: id.clone(),
                                            tab,
                                        },
                                        window,
                                        cx,
                                    )
                                }
                            })),
                    ),
                );
            }
            let mut header = div()
                .id(SharedString::from(format!("course-card-{id}")))
                .h(px(116.))
                .p_5()
                .bg(rgb(color))
                .cursor_pointer()
                .hover(|s| s.opacity(0.9))
                .child(
                    div()
                        .text_color(rgb(0xffffff))
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(field(course, "course_code")),
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.go(route.clone(), window, cx)),
                );
            // The card's rounded top corner clips its child header to a
            // rectangle, so the header carries the same radius itself.
            header.style().corner_radii.top_left = Some(px(6.).into());
            header.style().corner_radii.top_right = Some(px(6.).into());
            cards = cards.child(
                div()
                    .w(px(292.))
                    .flex_grow()
                    .max_w(px(390.))
                    .min_w(px(255.))
                    // A border on this rounded card leaves a stepped gray ring
                    // where the child header meets the corner. A shadow keeps
                    // the edge clean in both themes.
                    .shadow_sm()
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgb(colors.surface))
                    .child(header)
                    .child(
                        div()
                            .p_4()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(self.route_link(
                                format!("course-title-{id}"),
                                &model::course_title(course),
                                Route::Course {
                                    id: id.clone(),
                                    tab: CourseTab::Home,
                                },
                                cx,
                            ))
                            .child(muted(field(&course["term"], "name"), cx))
                            .child(shortcuts),
                    ),
            );
        }
        if visible.is_empty() {
            cards = cards.child(muted(
                "No current courses match your search. All Courses includes past enrollments.",
                cx,
            ));
        }
        div()
            .flex()
            .gap_7()
            .items_start()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(cards)
                    .child(self.route_button(
                        "all-courses",
                        "View all courses",
                        Route::Courses,
                        cx,
                    )),
            )
            .child(
                div()
                    .w(px(244.))
                    .flex_shrink_0()
                    .child(self.todo_sidebar(cx)),
            )
            .into_any_element()
    }
    fn todo_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let request = self.planner_request(true);
        let data = self.get(&request.name, request.args);
        let mut items = model::list(data, "items");
        items.sort_by_cached_key(|i| item_date(i));
        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("To Do"),
                    )
                    .child(
                        Button::new("sidebar-new-todo")
                            .icon(IconName::Plus)
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_note(None, None, window, cx)
                            })),
                    ),
            )
            .child(rule(cx));
        if data.is_none() {
            content = content.child(muted("Loading upcoming work…", cx));
        } else if items.is_empty() {
            content = content.child(muted("You’re all caught up.", cx));
        }
        for (index, item) in items.iter().take(7).enumerate() {
            content = content.child(self.planner_item(item, &format!("todo-{index}"), true, cx));
        }
        content
            .child(self.route_button("see-calendar", "View Calendar", Route::Calendar, cx))
            .into_any_element()
    }
    fn planner_item(
        &self,
        item: &Value,
        id: &str,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        let title = field(item, "title");
        let course = field(item, "course_id");
        let color = model::course_color(&course);
        let mut row = div()
            .flex()
            .gap_3()
            .py_3()
            .border_b_1()
            .border_color(rgb(colors.border));
        row = row.child(div().w(px(3.)).flex_shrink_0().bg(rgb(color)));
        let mut body = div().flex_1().min_w_0().flex().flex_col().gap_2();
        if let Some(route) = model::planner_route(item) {
            body = body.child(self.route_link(format!("{id}-open"), &title, route, cx));
        } else {
            body = body.child(div().font_weight(FontWeight::MEDIUM).child(title));
            if let Some(url) = item["html_url"].as_str() {
                body = body.child(self.web_link(format!("{id}-web"), "Open in Canvas", url));
            }
        }
        let course_name = if course.is_empty() {
            "Personal to-do".into()
        } else {
            self.course_name(&course)
        };
        body = body.child(muted(course_name, cx)).child(muted(
            model::date_label(
                item["due_at"]
                    .as_str()
                    .or(item["scheduled_at"].as_str())
                    .unwrap_or(""),
            ),
            cx,
        ));
        if !compact {
            body = body.child(badge(model::status(item), cx));
            let kind = field(item, "item_type");
            if matches!(
                kind.as_str(),
                "assignment" | "announcement" | "discussion_topic" | "wiki_page" | "quiz"
            ) && !course.is_empty()
            {
                let linked =
                    json!({"id":item["item_id"],"kind":kind,"title":item["title"],"course":course});
                body = body.child(
                    Button::new(SharedString::from(format!("{id}-reminder")))
                        .label("Add reminder")
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_note(None, Some(linked.clone()), window, cx)
                        })),
                );
            }
        }
        row.child(body).into_any_element()
    }
    fn all_courses(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        if self.get("course_list", json!({})).is_none() {
            return self.loading("course_list", json!({}), cx);
        }
        let query = self.search.read(cx).value().to_lowercase();
        let courses = self.courses();
        let mut view = div().flex().flex_col().gap_5().child(muted(
            "Select a course to open its Home page, Modules, Assignments, Grades, or Files.",
            cx,
        ));
        for (current, label) in [(true, "Current Enrollments"), (false, "Past Enrollments")] {
            let mut section = box_view(cx).child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(label),
            );
            let selected: Vec<_> = courses
                .iter()
                .filter(|c| {
                    model::course_is_current(c) == current
                        && c.to_string().to_lowercase().contains(&query)
                })
                .collect();
            if !current && !self.show_past {
                view = view.child(
                    Button::new("show-past-courses")
                        .label("Show past enrollments")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_past = true;
                            cx.notify();
                        })),
                );
                continue;
            }
            if selected.is_empty() {
                section = section.child(muted("No courses to display.", cx));
            }
            for course in selected {
                let id = field(course, "id");
                section = section.child(
                    div()
                        .flex()
                        .gap_5()
                        .items_center()
                        .py_4()
                        .border_t_1()
                        .border_color(rgb(colors.border))
                        .child(
                            div()
                                .size(px(12.))
                                .rounded_sm()
                                .bg(rgb(model::course_color(&id))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(self.route_link(
                                    format!("all-course-{id}"),
                                    &model::course_title(course),
                                    Route::Course {
                                        id,
                                        tab: CourseTab::Home,
                                    },
                                    cx,
                                ))
                                .child(muted(field(course, "course_code"), cx)),
                        )
                        .child(muted(field(&course["term"], "name"), cx)),
                );
            }
            view = view.child(section);
        }
        view.into_any_element()
    }
    fn course_home(
        &self,
        id: &str,
        syllabus_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(data) = self.get("course_info", json!({"course_id":id})) else {
            return self.loading("course_info", json!({"course_id":id}), cx);
        };
        let course = &data["course"];
        if !syllabus_only {
            match model::course_home_view(course) {
                "modules" => return self.modules(id, cx),
                "assignments" => return self.assignments(id, cx),
                "wiki" => {
                    let Some(front) = self.get("course_front_page", json!({"course_id":id})) else {
                        return self.loading("course_front_page", json!({"course_id":id}), cx);
                    };
                    let page = &front["page"];
                    return div().w_full().min_w_0().flex().flex_col().gap_5()
                        .child(heading(field(page,"title")))
                        .child(self.canvas_body(format!("front-page-{id}"), &field(page,"body_html"), id, window, cx))
                        .into_any_element();
                },
                "feed" => return box_view(cx).child(heading("Course activity"))
                    .child(muted("This course uses Canvas’s activity stream. Open it in Canvas to see discussions and recent activity.",cx))
                    .child(self.web_link("home-activity", "Open course activity in Canvas", &format!("{}/courses/{id}",self.settings.origin.trim_end_matches('/')))).into_any_element(),
                _ => {},
            }
        }
        let mut view = div().flex().flex_col().gap_6();
        if !syllabus_only && model::course_home_view(course) != "syllabus" {
            view = view
                .child(heading(model::course_title(course)))
                .child(muted(
                    format!(
                        "{}  ·  {}",
                        field(course, "course_code"),
                        field(&course["term"], "name")
                    ),
                    cx,
                ));
            let teachers = model::list(Some(course), "teachers")
                .iter()
                .map(|v| field(v, "display_name"))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            if !teachers.is_empty() {
                view = view.child(muted(format!("Instructor: {teachers}"), cx));
            }
            view = view.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(self.route_button(
                        "home-modules",
                        "View Modules",
                        Route::Course {
                            id: id.into(),
                            tab: CourseTab::Modules,
                        },
                        cx,
                    ))
                    .child(self.route_button(
                        "home-assignments",
                        "View Assignments",
                        Route::Course {
                            id: id.into(),
                            tab: CourseTab::Assignments,
                        },
                        cx,
                    ))
                    .child(self.route_button(
                        "home-grades",
                        "View Grades",
                        Route::Course {
                            id: id.into(),
                            tab: CourseTab::Grades,
                        },
                        cx,
                    )),
            );
        }
        let html = field(course, "syllabus_body_html");
        let text = crate::html_content::notice_text(&html);
        if text.trim().is_empty() || text.trim().eq_ignore_ascii_case("syllabus") {
            view = view.child(box_view(cx)
                .child(muted("This course’s syllabus page is empty. Your instructor may have posted the syllabus as a file in Modules.", cx))
                .child(self.route_button("syllabus-modules", "Browse Modules", Route::Course{id:id.into(),tab:CourseTab::Modules},cx)));
        } else {
            view = view.child(self.canvas_body(format!("syllabus-{id}"), &html, id, window, cx));
        }
        view.into_any_element()
    }
    fn modules(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        if self.get("module_list", json!({"course_id":id})).is_none() {
            return self.loading("module_list", json!({"course_id":id}), cx);
        }
        if self.module_rows.is_empty() {
            return muted("No modules or module items match this view.", cx).into_any_element();
        }
        let view = cx.entity().downgrade();
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(
                gpui::list(self.module_scroll.clone(), move |index, _, cx| {
                    view.update(cx, |this, cx| this.module_row(index, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .size_full(),
            )
            .child(gpui_component::scroll::Scrollbar::vertical(
                &self.module_scroll,
            ))
            .into_any_element()
    }
    fn module_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        use crate::module_list::{ModuleRow, ModuleTarget};
        let Some(row) = self.module_rows.get(index).cloned() else {
            return div().into_any_element();
        };
        match row {
            ModuleRow::Header {
                key,
                title,
                state,
                expanded,
            } => module_row_container()
                .when(index > 0, |d| d.pt_4())
                .child(
                    div()
                        .id(SharedString::from(format!("module-header-{key}")))
                        .w_full()
                        .min_w_0()
                        .flex()
                        .gap_3()
                        .items_center()
                        .px_5()
                        .py_4()
                        .border_1()
                        .border_color(rgb(colors.border))
                        .bg(rgb(colors.subtle))
                        .cursor_pointer()
                        .child(icon_slot(
                            if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            rgb(colors.muted),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(muted(state, cx))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.expanded_modules.insert(key.clone(), !expanded);
                            this.rebuild_module_rows(cx);
                            cx.notify();
                        })),
                )
                .into_any_element(),
            ModuleRow::Item {
                key,
                title,
                kind,
                target,
                due,
                lock,
                completed,
                indent,
            } => {
                let icon = match kind.as_str() {
                    "Page" => IconName::BookOpen,
                    "Quiz" => IconName::CircleCheck,
                    "Discussion" => IconName::Inbox,
                    "ExternalUrl" | "ExternalTool" => IconName::ExternalLink,
                    _ => IconName::File,
                };
                let mut body = div().flex_1().min_w_0().flex().flex_col().gap_2();
                body = body.child(match target {
                    ModuleTarget::Native(route) => self.route_link(key.clone(), &title, route, cx),
                    ModuleTarget::Web(url) => self.web_text_link(key.clone(), &title, &url, cx),
                    ModuleTarget::None => div()
                        .when(kind == "SubHeader", |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(title)
                        .into_any_element(),
                });
                if let Some(due) = due {
                    body = body.child(muted(format!("Due {due}"), cx));
                }
                if let Some(lock) = lock {
                    body = body.child(muted(lock, cx));
                }
                module_row_container()
                    .flex()
                    .items_start()
                    .gap_4()
                    .px_5()
                    .py_4()
                    .border_l_1()
                    .border_r_1()
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(div().flex_shrink_0().ml(px(indent as f32 * 16.)).child(
                        if kind == "SubHeader" {
                            div().size(px(24.)).into_any_element()
                        } else {
                            icon_slot(icon, rgb(colors.muted)).into_any_element()
                        },
                    ))
                    .child(body)
                    .when(completed, |d| {
                        d.child(icon_slot(IconName::CircleCheck, rgb(colors.success)))
                    })
                    .into_any_element()
            }
        }
    }
    fn assignments(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(data) = self.get("assignment_list", json!({"course_id":id})) else {
            return self.loading("assignment_list", json!({"course_id":id}), cx);
        };
        let mut view = div().flex().flex_col().gap_5();
        let mut filters = div().flex().gap_2();
        for label in ["All", "Upcoming", "Missing", "Submitted"] {
            filters = filters.child(
                Button::new(SharedString::from(format!("assignment-filter-{label}")))
                    .label(label)
                    .small()
                    .when(self.assignment_filter == label, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.assignment_filter = label.into();
                        cx.notify();
                    })),
            );
        }
        view = view.child(filters);
        let query = self.search.read(cx).value().to_lowercase();
        let today = Local::now().date_naive();
        let mut items = model::list(Some(data), "assignments");
        items.sort_by_key(|v| (field(v, "due_at").is_empty(), field(v, "due_at")));
        let selected: Vec<_> = items
            .into_iter()
            .filter(|v| field(v, "name").to_lowercase().contains(&query))
            .filter(|v| match self.assignment_filter.as_str() {
                "Missing" => model::status(v) == "Missing",
                "Submitted" => matches!(model::status(v), "Submitted" | "Graded"),
                "Upcoming" => v["due_at"]
                    .as_str()
                    .and_then(model::date)
                    .is_some_and(|d| d >= today),
                _ => true,
            })
            .collect();
        let mut list = div()
            .border_1()
            .border_color(rgb(colors.border))
            .rounded_sm();
        if selected.is_empty() {
            list = list.child(
                div()
                    .p_5()
                    .child(muted("No assignments match this view.", cx)),
            );
        }
        for assignment in selected {
            let aid = field(assignment, "id");
            list = list.child(
                div()
                    .px_5()
                    .py_4()
                    .flex()
                    .gap_4()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(Icon::new(IconName::File).text_color(rgb(colors.success)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(self.route_link(
                                format!("assignment-{aid}"),
                                &field(assignment, "name"),
                                Route::Assignment {
                                    course: id.into(),
                                    id: aid,
                                },
                                cx,
                            ))
                            .child(muted(
                                format!(
                                    "{}  ·  {} points",
                                    model::date_label(assignment["due_at"].as_str().unwrap_or("")),
                                    field(assignment, "points_possible")
                                ),
                                cx,
                            )),
                    )
                    .child(badge(model::status(assignment), cx)),
            );
        }
        view.child(list).into_any_element()
    }
    fn assignment_detail(
        &self,
        course: &str,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let args = json!({"course_id":course,"assignment_id":id});
        let Some(data) = self.get("assignment_info", args.clone()) else {
            return self.loading("assignment_info", args, cx);
        };
        let assignment = &data["assignment"];
        let mut view = div()
            .flex()
            .flex_col()
            .gap_6()
            .child(heading(field(assignment, "name")));
        let mut meta = div()
            .flex()
            .flex_wrap()
            .gap_5()
            .child(muted(
                format!(
                    "Due {}",
                    model::date_label(assignment["due_at"].as_str().unwrap_or(""))
                ),
                cx,
            ))
            .child(muted(
                format!("{} points", field(assignment, "points_possible")),
                cx,
            ))
            .child(badge(model::status(assignment), cx));
        let types = assignment["submission_types"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.replace('_', " "))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        if !types.is_empty() {
            meta = meta.child(muted(format!("Submitting: {types}"), cx));
        }
        view = view.child(meta);
        let mut actions = div().flex().flex_wrap().gap_3();
        if let Some(url) = assignment["html_url"].as_str() {
            actions = actions.child(self.web_link("submit-online", "Open / Submit in Canvas", url));
        }
        let linked = json!({"id":id,"kind":"assignment","title":field(assignment,"name")});
        actions = actions.child(
            Button::new("assignment-reminder")
                .label("Add to To Do")
                .small()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_note(None, Some(linked.clone()), window, cx)
                })),
        );
        view = view.child(actions).child(rule(cx));
        if let Some(reason) = assignment["lock_explanation"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            view =
                view.child(box_view(cx).child(muted(crate::html_content::notice_text(reason), cx)));
        }
        let html = field(assignment, "description_html");
        if html.is_empty() {
            view = view.child(muted(
                if assignment["locked_for_user"] == true {
                    "Instructions will be available when this assignment is unlocked."
                } else {
                    "No instructions have been provided."
                },
                cx,
            ));
        } else {
            view = view.child(self.canvas_body(
                format!("assignment-body-{course}-{id}"),
                &html,
                course,
                window,
                cx,
            ));
        }
        let attachments = model::list(Some(assignment), "attachments");
        if !attachments.is_empty() {
            let mut files =
                box_view(cx).child(div().font_weight(FontWeight::SEMIBOLD).child("Attachments"));
            for (index, file) in attachments.iter().enumerate() {
                files = files.child(self.attachment_row(
                    file,
                    Some(course),
                    &format!("assignment-file-{index}"),
                    cx,
                ));
            }
            view = view.child(files);
        }
        let grade = self.get(
            "assignment_grade",
            json!({"course_id":course,"assignment_id":id}),
        );
        let grade = grade
            .and_then(|v| {
                v.get("assignment_grade").or_else(|| {
                    v["course_grades"]["assignments"]
                        .as_array()
                        .and_then(|a| a.first())
                })
            })
            .filter(|v| !v.is_null())
            .unwrap_or(&assignment["submission"]);
        let score = if grade["score"].is_null() {
            "Not graded".into()
        } else {
            format!(
                "{} / {}",
                field(grade, "score"),
                field(assignment, "points_possible")
            )
        };
        view = view.child(
            box_view(cx)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Submission & Grade"),
                )
                .child(div().text_xl().child(score))
                .child(muted(model::status(assignment), cx))
                .when(grade["graded_at"].as_str().is_some(), |d| {
                    d.child(muted(
                        format!(
                            "Graded {}",
                            model::date_label(grade["graded_at"].as_str().unwrap_or(""))
                        ),
                        cx,
                    ))
                }),
        );
        view.into_any_element()
    }
    fn attachment_row(
        &self,
        file: &Value,
        course: Option<&str>,
        id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        let title = file["display_name"]
            .as_str()
            .or(file["filename"].as_str())
            .unwrap_or("Attachment")
            .to_owned();
        let resource = file["resource"]
            .as_str()
            .or(file["download_resource"].as_str())
            .unwrap_or("")
            .to_owned();
        let mut row = div()
            .flex()
            .gap_3()
            // Align the icon with the attachment name (the first line), not
            // with the centre of the name-and-size block.
            .items_start()
            .py_3()
            .border_t_1()
            .border_color(rgb(colors.border))
            .child(icon_slot(IconName::File, rgb(colors.muted)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(self.document_link(
                        format!("{id}-preview"),
                        &title,
                        DocumentSelection {
                            course: course.map(str::to_owned),
                            resource,
                            title: title.clone(),
                            start: 0,
                        },
                        cx,
                    ))
                    .child(muted(
                        file["size"]
                            .as_u64()
                            .map(|n| format!("{:.1} KB", n as f64 / 1024.))
                            .unwrap_or_default(),
                        cx,
                    )),
            );
        for (save_as, label) in [(false, "Download"), (true, "Save As…")] {
            let file = file.clone();
            row = row.child(
                Button::new(SharedString::from(format!("{id}-{save_as}")))
                    .label(label)
                    .small()
                    .disabled(self.demo)
                    .on_click(cx.listener(move |this, _, _, cx| this.download(&file, save_as, cx))),
            );
        }
        row.into_any_element()
    }

    fn grades(&self, course: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(id) = course else {
            let Some(data) = self.get("grade_summary", json!({})) else {
                return self.loading("grade_summary", json!({}), cx);
            };
            let mut view = div().flex().flex_col().gap_4().child(muted(
                "Select a course to see individual assignment scores and submission status.",
                cx,
            ));
            for grade in model::list(Some(data), "grades") {
                let id = field(grade, "course_id");
                view = view.child(
                    box_view(cx).child(
                        div()
                            .flex()
                            .items_center()
                            .gap_5()
                            .child(div().w(px(4.)).h(px(55.)).bg(rgb(model::course_color(&id))))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(self.route_link(
                                        format!("grade-course-{id}"),
                                        &field(grade, "course_name"),
                                        Route::Course {
                                            id,
                                            tab: CourseTab::Grades,
                                        },
                                        cx,
                                    ))
                                    .child(muted(field(grade, "course_code"), cx)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .child(model::score(&grade["current_score"])),
                                    )
                                    .child(muted(
                                        format!("Current grade {}", field(grade, "current_grade")),
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(muted("Final grade", cx))
                                    .child(format!(
                                        "{} {}",
                                        model::score(&grade["final_score"]),
                                        field(grade, "final_grade")
                                    )),
                            ),
                    ),
                );
            }
            return view.into_any_element();
        };
        let Some(data) = self.get("course_grades", json!({"course_id":id})) else {
            return self.loading("course_grades", json!({"course_id":id}), cx);
        };
        let grades = &data["course_grades"];
        let mut view = div().flex().flex_col().gap_6().child(
            box_view(cx).child(
                div()
                    .flex()
                    .gap_8()
                    .child(div().flex_1().child(muted("Current grade", cx)).child(
                        div().text_3xl().child(format!(
                            "{} {}",
                            model::score(&grades["total_grade"]["current_score"]),
                            field(&grades["total_grade"], "current_grade")
                        )),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .child(muted("Final grade (including ungraded work)", cx))
                            .child(div().text_2xl().child(format!(
                                "{} {}",
                                model::score(&grades["total_grade"]["final_score"]),
                                field(&grades["total_grade"], "final_grade")
                            ))),
                    ),
            ),
        );
        let mut table = div()
            .border_1()
            .border_color(rgb(colors.border))
            .rounded_sm()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .p_4()
                    .bg(rgb(colors.subtle))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().flex_1().child("Assignment"))
                    .child(div().w(px(130.)).child("Score"))
                    .child(div().w(px(110.)).child("Status")),
            );
        let query = self.search.read(cx).value().to_lowercase();
        for grade in model::list(Some(grades), "assignments")
            .into_iter()
            .filter(|v| field(v, "name").to_lowercase().contains(&query))
        {
            let aid = field(grade, "assignment_id");
            let score = if grade["score"].is_null() {
                "—".into()
            } else {
                format!(
                    "{} / {}",
                    field(grade, "score"),
                    field(grade, "points_possible")
                )
            };
            table = table.child(
                div()
                    .flex()
                    .gap_4()
                    .p_4()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(colors.border))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(self.route_link(
                                format!("grade-assignment-{aid}"),
                                &field(grade, "name"),
                                Route::Assignment {
                                    course: id.into(),
                                    id: aid,
                                },
                                cx,
                            ))
                            .child(muted(
                                model::date_label(grade["due_at"].as_str().unwrap_or("")),
                                cx,
                            )),
                    )
                    .child(div().w(px(130.)).flex_shrink_0().child(score))
                    .child(
                        div()
                            .w(px(110.))
                            .flex_shrink_0()
                            .child(badge(model::status(grade), cx)),
                    ),
            );
        }
        view = view.child(table);
        view.into_any_element()
    }
    fn files(&self, course: &str, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(data) = self.get("module_list", json!({"course_id":course})) else {
            return self.loading("module_list", json!({"course_id":course}), cx);
        };
        let mut view=div().flex().flex_col().gap_5().child(muted("Files shared in course modules. Files attached to assignments are available on each assignment page.", cx));
        let query = self.search.read(cx).value().to_lowercase();
        let mut count = 0;
        for module in model::list(Some(data), "modules") {
            let items = model::list(Some(module), "items")
                .into_iter()
                .filter(|v| {
                    field(v, "item_type") == "File"
                        && field(v, "title").to_lowercase().contains(&query)
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                continue;
            }
            let mut folder = box_view(cx).child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(icon_slot(IconName::Folder, rgb(colors.muted)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(field(module, "name")),
                    ),
            );
            for file in items {
                let id = field(file, "content_id");
                folder = folder.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_3()
                        .border_t_1()
                        .border_color(rgb(colors.border))
                        .child(icon_slot(IconName::File, rgb(colors.muted)))
                        .child(div().flex_1().min_w_0().child(self.file_document_link(
                            format!("file-{id}"),
                            &field(file, "title"),
                            course,
                            id,
                            cx,
                        ))),
                );
                count += 1;
            }
            view = view.child(folder);
        }
        if count == 0 {
            view = view.child(muted("No module files match this view.", cx));
        }
        view.into_any_element()
    }
    fn file_detail(
        &self,
        course: &str,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(data) = self.get("file_info", json!({"file_id":id})) else {
            return self.loading("file_info", json!({"file_id":id}), cx);
        };
        let file = &data["file"];
        let mut view = box_view(cx)
            .child(heading(
                file["display_name"]
                    .as_str()
                    .or(file["filename"].as_str())
                    .unwrap_or("File")
                    .to_owned(),
            ))
            .child(muted(
                format!(
                    "{} · {}",
                    field(file, "content_type"),
                    file["size"]
                        .as_u64()
                        .map(|n| format!("{:.1} KB", n as f64 / 1024.))
                        .unwrap_or_default()
                ),
                cx,
            ))
            .child(self.attachment_row(file, Some(course), "file-detail", cx));
        if self.show_technical
            && let Some(uri) = file["download_resource"].as_str()
        {
            let uri = uri.to_owned();
            view = view.child(
                Button::new("preview-original-file")
                    .label("Preview original")
                    .small()
                    .on_click(cx.listener({
                        let uri = uri.clone();
                        move |this, _, _, cx| this.perform(Operation::Resource(uri.clone()), cx)
                    })),
            );
            if let Some(preview) = self.data.get(&format!("resource:{uri}")) {
                if let Some(image) = self.resource_images.get(&uri) {
                    view = view.child(
                        img(image.clone())
                            .max_w_full()
                            .max_h(px(560.))
                            .object_fit(ObjectFit::Contain),
                    );
                } else if let Some(text) = preview["contents"][0]["text"].as_str() {
                    view = view.child(muted(text.to_owned(), cx));
                } else {
                    view=view.child(muted("Download this file to view it in its default application, or select its name above to read the document text.", cx));
                }
            }
        }
        if let Some(reason) = file["lock_explanation"].as_str().filter(|s| !s.is_empty()) {
            view = view.child(muted(crate::html_content::notice_text(reason), cx));
        }
        if let Some(document) = self.active_document() {
            view = view.child(self.document_viewer(&document, ReaderLayout::Embedded, window, cx));
        }
        view.into_any_element()
    }
    fn document(
        &self,
        course: &Option<String>,
        resource: &str,
        title: &str,
        start: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.document_viewer(
            &DocumentSelection {
                course: course.clone(),
                resource: resource.into(),
                title: title.into(),
                start,
            },
            ReaderLayout::Route,
            window,
            cx,
        )
    }
    fn document_viewer(
        &self,
        document: &DocumentSelection,
        layout: ReaderLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        let mut view = div().w_full().min_w_0().flex().flex_col().gap_2();
        if layout == ReaderLayout::Embedded {
            let file = json!({"resource":document.resource,"download_resource":document.resource,"filename":document.title});
            view = view.child(self.attachment_row(
                &file,
                document.course.as_deref(),
                "viewer-file",
                cx,
            ));
        }
        let request = model::document_preview_request(
            &document.resource,
            self.document_page,
            self.document_zoom,
        );
        let key = model::cache_key(&request.name, &request.args);
        let data = self.get(&request.name, request.args.clone());
        let text_only = data.is_some_and(|value| value["document"]["kind"] == "text");
        let known_pages = self.document_page_count();
        // One compact bar replaces the former title row, mode tabs, page
        // controls and file actions. The page list gets everything else.
        let mut bar = div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(rgb(colors.border));
        for (label, mode) in [
            ("Document", DocumentView::Pages),
            ("Text & images", DocumentView::Text),
        ] {
            bar = bar.child(
                Button::new(SharedString::from(format!("document-mode-{label}")))
                    .label(label)
                    .small()
                    .disabled(mode == DocumentView::Pages && text_only)
                    .when(self.document_view == mode, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_document_view(mode, cx))),
            );
        }
        if self.document_view == DocumentView::Pages
            && let Some(pages) = known_pages
        {
            let document_for_window = document.clone();
            bar = bar
                .child(div().flex_1())
                .child(
                    Button::new("pdf-previous")
                        .icon(IconName::ChevronLeft)
                        .small()
                        .disabled(self.document_page <= 1)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_document_page(this.document_page.saturating_sub(1), cx)
                        })),
                )
                .child(div().w(px(56.)).child(Input::new(&self.document_jump)))
                .child(
                    Button::new("pdf-go")
                        .label("Go")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            match this.document_jump.read(cx).value().parse::<u32>() {
                                Ok(page) if page > 0 => this.set_document_page(page, cx),
                                _ => {
                                    this.error = Some("Enter a page number starting at 1.".into());
                                    cx.notify();
                                }
                            }
                        })),
                )
                .child(muted(format!("of {pages}"), cx))
                .child(
                    Button::new("pdf-next")
                        .icon(IconName::ChevronRight)
                        .small()
                        .disabled(self.document_page >= pages)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_document_page(this.document_page.saturating_add(1), cx)
                        })),
                )
                .child(
                    Button::new("pdf-zoom-out")
                        .icon(IconName::Minus)
                        .small()
                        .disabled(self.document_zoom <= model::DOCUMENT_MIN_ZOOM)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_document_zoom(this.document_zoom.saturating_sub(25), cx)
                        })),
                )
                .child(muted(format!("{}%", self.document_zoom), cx))
                .child(
                    Button::new("pdf-zoom-in")
                        .icon(IconName::Plus)
                        .small()
                        .disabled(self.document_zoom >= model::DOCUMENT_MAX_ZOOM)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_document_zoom(this.document_zoom.saturating_add(25), cx)
                        })),
                )
                .child(
                    Button::new("pdf-fit")
                        .label("Fit width")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.set_document_zoom(100, cx))),
                )
                .child(
                    Button::new("pdf-fit-page")
                        .label("Fit page")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.fit_document_page(cx))),
                );
            if layout == ReaderLayout::Route {
                bar = bar.child(
                    Button::new("document-open-window")
                        .label("Open in window")
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_document_window(document_for_window.clone(), cx)
                        })),
                );
            }
        }
        view = view.child(bar);
        if self.document_view == DocumentView::Text {
            return view
                .child(self.document_text(
                    &document.course,
                    &document.resource,
                    &document.title,
                    document.start,
                    window,
                    cx,
                ))
                .into_any_element();
        }
        if data.is_none() && known_pages.is_none() {
            if !self.failures.contains_key(&key) {
                return view
                    .child(self.loading(&request.name, request.args.clone(), cx))
                    .into_any_element();
            }
            return view
                .child(
                    box_view(cx).child(muted(
                        "The document preview could not be opened. Try Text & images, or download the original.",
                        cx,
                    )),
                )
                .into_any_element();
        }
        if text_only {
            return view
                .child(muted(
                    "This document is available in Text & images mode.",
                    cx,
                ))
                .into_any_element();
        }
        // Window and route readers fill the remaining height. The embedded
        // reader keeps a fixed reading height inside a scrolling file screen.
        view = if layout == ReaderLayout::Embedded {
            view
        } else {
            view.flex_1().min_h_0()
        };
        view.child(self.document_pages(layout == ReaderLayout::Embedded, cx))
            .into_any_element()
    }
    fn document_pages(&self, fixed_height: bool, cx: &mut Context<Self>) -> AnyElement {
        if self.document_scroll.item_count() == 0 {
            return muted("Preparing pages…", cx).into_any_element();
        }
        // Item renderers run while the list holds a mutable borrow of its own
        // state, so the viewport must be read here and captured by the closure
        // instead of from inside a row.
        let measured = f32::from(self.document_scroll.viewport_bounds().size.width);
        let available = if measured > 200.0 {
            measured
        } else {
            model::DOCUMENT_PAGE_WIDTH + 32.0
        };
        let view = cx.entity().downgrade();
        // `flex_1()` sets a zero flex-basis. `flex_none()` does not reset the
        // basis, so applying both and then an explicit height would collapse
        // the list to zero height. Choose one sizing mode instead.
        let mut pages = div().relative().w_full();
        pages = if fixed_height {
            pages.h(px(760.))
        } else {
            pages.flex_1().min_h_0()
        };
        pages
            .child(
                gpui::list(self.document_scroll.clone(), move |index, _, cx| {
                    view.update(cx, |this, cx| this.document_page_row(index, available, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .size_full(),
            )
            .child(gpui_component::scroll::Scrollbar::vertical(
                &self.document_scroll,
            ))
            .into_any_element()
    }
    fn document_page_row(
        &self,
        index: usize,
        available: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = palette(cx);
        let Some(document) = self.active_document() else {
            return div().into_any_element();
        };
        let page = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
        // 100% fits a page to the reader column; larger zooms widen the page
        // beyond it so the row can scroll sideways.
        let column = (available - 12.0).max(240.0);
        let width = column * self.document_zoom as f32 / 100.0;
        let request = model::document_preview_request(&document.resource, page, self.document_zoom);
        let key = model::cache_key(&request.name, &request.args);
        let metadata = self.data.get(&key).map(|value| &value["document"]);
        // Page geometry is known once a raster exists. Until then a portrait
        // page keeps the scrollbar stable instead of collapsing the row.
        let aspect = metadata
            .and_then(|document| {
                Some(document["image_height"].as_f64()? / document["image_width"].as_f64()?)
            })
            .filter(|aspect| aspect.is_finite() && *aspect > 0.0)
            .map(|aspect| aspect as f32)
            .unwrap_or(std::f32::consts::SQRT_2);
        let height = width * aspect;
        let image = self.document_images.get(&key).cloned();
        let failed = self.failures.contains_key(&key);
        let padding = if available > width + 12. {
            (available - width) / 2.0
        } else {
            6.0
        };
        div()
            .w_full()
            .py_1()
            .flex()
            .items_center()
            .child(
                page_side_scroll(
                    div()
                        .id(SharedString::from(format!("document-page-scroll-{page}")))
                        .w_full()
                        .overflow_x_scroll(),
                )
                .child(
                    div()
                        .ml(px(padding))
                        .mr(px(padding))
                        .w(px(width))
                        .h(px(height))
                        .flex()
                        .items_center()
                        .justify_center()
                        // Paper stays white in both themes; only the reader
                        // chrome follows the appearance setting.
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(colors.border))
                        .child(match (image, failed) {
                            (Some(image), _) => img(image)
                                .size_full()
                                .object_fit(ObjectFit::Contain)
                                .into_any_element(),
                            (None, true) => div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(muted("This page could not be rendered.", cx))
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "retry-document-page-{page}"
                                    )))
                                    .label("Retry")
                                    .small()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.retry_document_page(page, cx)
                                    })),
                                )
                                .into_any_element(),
                            (None, false) => loading_spinner(cx),
                        }),
                ),
            )
            .into_any_element()
    }
    fn document_text(
        &self,
        course: &Option<String>,
        resource: &str,
        title: &str,
        start: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let args = json!({"resource":resource,"start":start});
        let Some(data) = self.get("attachment_text", args.clone()) else {
            return self.loading("attachment_text", args, cx);
        };
        let mut view = div().w_full().min_w_0().flex().flex_col().gap_5();
        let text = field(data, "text");
        view = view.child(box_view(cx).child(
            TextView::markdown("document-text", escape_text(&text), window, cx).selectable(true),
        ));
        let mut pages = div().flex().gap_3();
        if start > 0 {
            pages = pages.child(self.route_button(
                "document-first",
                "Back to beginning",
                Route::Document {
                    course: course.clone(),
                    resource: resource.into(),
                    title: title.into(),
                    start: 0,
                },
                cx,
            ));
        }
        if let Some(next) = data["next_start"].as_u64() {
            pages = pages.child(self.route_button(
                "document-next",
                "Continue reading",
                Route::Document {
                    course: course.clone(),
                    resource: resource.into(),
                    title: title.into(),
                    start: next,
                },
                cx,
            ));
        }
        view = view.child(pages);
        if let Some(images) = data["images"].as_array().filter(|a| !a.is_empty()) {
            let mut figures = box_view(cx).child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Images in this document"),
            );
            for (index, image) in images.iter().enumerate() {
                let resource = resource.to_owned();
                let page = image["page"].clone();
                let number = image["image"].clone();
                let supported = image["supported"].as_bool() == Some(true);
                figures=figures.child(div().flex().items_center().gap_3().child(Button::new(SharedString::from(format!("figure-{index}"))).label(format!("Page {page} · image {number}")).small().disabled(!supported)
                    .on_click(cx.listener(move|this,_,_,cx|this.perform(Operation::Tool{name:"attachment_image".into(),args:json!({"resource":resource,"page":page,"image":number})},cx))))
                    .child(muted(if supported{format!("{} × {}",image["width"],image["height"])}else{field(image,"unsupported_reason")}, cx)));
            }
            view = view.child(figures);
        }
        if let Some(image) = &self.image {
            view = view.child(
                box_view(cx).child(
                    img(image.clone())
                        .max_w_full()
                        .max_h(px(600.))
                        .object_fit(ObjectFit::Contain),
                ),
            );
        }
        if self.image_content.is_some() {
            view = view.child(
                Button::new("save-figure")
                    .label("Save image…")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.save_image(cx))),
            );
            if self.image.is_none() {
                view=view.child(muted("This image format cannot be previewed here. Save it to open in another application.", cx));
            }
        }
        view.into_any_element()
    }
    fn page_detail(
        &self,
        course: &str,
        slug: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let args = json!({"course_id":course,"page_url":slug});
        let Some(data) = self.get("page_info", args.clone()) else {
            return self.loading("page_info", args, cx);
        };
        let page = &data["page"];
        let mut view = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(heading(field(page, "title")));
        if let Some(reason) = page["lock_explanation"].as_str().filter(|s| !s.is_empty()) {
            view = view.child(muted(crate::html_content::notice_text(reason), cx));
        }
        view = view.child(self.canvas_body(
            format!("page-body-{slug}"),
            &field(page, "body_html"),
            course,
            window,
            cx,
        ));
        if let Some(url) = page["html_url"].as_str() {
            view = view.child(self.web_link("page-online", "Open page in Canvas", url));
        }
        view.into_any_element()
    }
    fn note_detail(&self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let args = json!({"note_id":id});
        let Some(data) = self.get("planner_note_info", args.clone()) else {
            return self.loading("planner_note_info", args, cx);
        };
        let note = &data["note"];
        let edit = note.clone();
        let delete = id.to_owned();
        let mut view = box_view(cx)
            .child(heading(field(note, "title")))
            .child(muted(
                model::date_label(note["todo_date"].as_str().unwrap_or("")),
                cx,
            ));
        if let Some(course) = note["course_id"].as_str() {
            view = view.child(muted(self.course_name(course), cx));
        }
        view.child(
            TextView::markdown(
                "note-details",
                escape_text(&field(note, "details")),
                window,
                cx,
            )
            .selectable(true),
        )
        .child(
            div()
                .flex()
                .gap_3()
                .child(
                    Button::new("edit-todo")
                        .label("Edit To-do")
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_note(Some(&edit), None, window, cx)
                        })),
                )
                .child(
                    Button::new("delete-todo")
                        .label("Delete")
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_delete = Some(delete.clone());
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
    }
    fn calendar(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let request = self.planner_request(false);
        let data = self.get(&request.name, request.args.clone());
        let items = model::list(data, "items");
        let mut view = div().flex().flex_col().gap_5();
        let mut controls = div().flex().flex_wrap().gap_3().items_center();
        for (delta, icon) in [(-1, IconName::ChevronLeft), (1, IconName::ChevronRight)] {
            controls = controls.child(
                Button::new(SharedString::from(format!("month-{delta}")))
                    .icon(icon)
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.month = model::shift_month(this.month, delta);
                        this.applied_range = None;
                        this.calendar_day = None;
                        this.load_route(cx);
                        cx.notify();
                    })),
            );
        }
        controls = controls
            .child(
                Button::new("calendar-today")
                    .label("Today")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.month = model::month_start(Local::now().date_naive());
                        this.applied_range = None;
                        this.calendar_day = None;
                        this.load_route(cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xl()
                    .flex_1()
                    .child(self.month.format("%B %Y").to_string()),
            )
            .child(
                Button::new("calendar-format")
                    .label(if self.calendar_list {
                        "Month"
                    } else {
                        "Agenda"
                    })
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.calendar_list = !this.calendar_list;
                        this.applied_range = None;
                        this.calendar_day = None;
                        this.load_route(cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("calendar-new-todo")
                    .icon(IconName::Plus)
                    .label("To-do")
                    .small()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_note(None, None, window, cx)),
                    ),
            );
        view = view.child(controls);
        let course_label = self
            .calendar_course
            .as_deref()
            .map(|id| self.course_name(id))
            .unwrap_or_else(|| "All courses".into());
        let mut filters = div().flex().flex_wrap().gap_2().items_center().child(
            Button::new("calendar-course-picker")
                .label(course_label)
                .icon(IconName::ChevronDown)
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.picker = if this.picker == Some(Picker::CalendarCourse) {
                        None
                    } else {
                        Some(Picker::CalendarCourse)
                    };
                    cx.notify();
                })),
        );
        for (label, filter) in [
            ("All", None),
            ("To Do", Some("incomplete_items")),
            ("Completed", Some("complete_items")),
            ("New activity", Some("new_activity")),
        ] {
            filters = filters.child(
                Button::new(SharedString::from(format!("calendar-filter-{label}")))
                    .label(label)
                    .small()
                    .when(self.calendar_filter.as_deref() == filter, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.calendar_filter = filter.map(str::to_owned);
                        this.load_route(cx);
                        cx.notify();
                    })),
            );
        }
        view = view.child(filters);
        if self.picker == Some(Picker::CalendarCourse) {
            view = view.child(self.course_picker(Picker::CalendarCourse, cx));
        }
        if self.calendar_list {
            view = view.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(muted("From", cx))
                    .child(div().w(px(150.)).child(Input::new(&self.range_start)))
                    .child(muted("Through", cx))
                    .child(div().w(px(150.)).child(Input::new(&self.range_end)))
                    .child(
                        Button::new("apply-date-range")
                            .label("Apply dates")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let start = this.range_start.read(cx).value().to_string();
                                let end = this.range_end.read(cx).value().to_string();
                                if let (Ok(a), Ok(b)) = (
                                    NaiveDate::parse_from_str(&start, "%Y-%m-%d"),
                                    NaiveDate::parse_from_str(&end, "%Y-%m-%d"),
                                ) && a <= b
                                {
                                    this.applied_range = Some((start, end));
                                    this.calendar_day = None;
                                    this.load_route(cx);
                                } else {
                                    this.error = Some(
                                        "Choose valid dates with the end on or after the start."
                                            .into(),
                                    );
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        if data.is_none() {
            return view
                .child(self.loading(&request.name, request.args, cx))
                .into_any_element();
        }
        let query = self.search.read(cx).value().to_lowercase();
        let mut items: Vec<_> = items
            .into_iter()
            .filter(|i| field(i, "title").to_lowercase().contains(&query))
            .collect();
        items.sort_by_cached_key(|i| item_date(i));
        if self.calendar_list || self.calendar_day.is_some() {
            if let Some(day) = self.calendar_day {
                view = view.child(
                    div()
                        .flex()
                        .gap_4()
                        .items_center()
                        .child(heading(day.format("%A, %B %-d").to_string()))
                        .child(
                            Button::new("calendar-all-days")
                                .label("Back to month")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.calendar_day = None;
                                    cx.notify();
                                })),
                        ),
                );
            }
            let selected: Vec<_> = items
                .iter()
                .filter(|i| {
                    self.calendar_day
                        .is_none_or(|day| item_date(i) == Some(day))
                })
                .collect();
            if selected.is_empty() {
                view = view.child(muted("Nothing scheduled in this view.", cx));
            }
            for (index, item) in selected.iter().enumerate() {
                view = view.child(self.planner_item(item, &format!("agenda-{index}"), false, cx));
            }
        } else {
            let first = self.month
                - chrono::Duration::days(self.month.weekday().num_days_from_sunday() as i64);
            let items_by_day = model::planner_items_by_date(items.iter().copied());
            let today = Local::now().date_naive();
            let mut grid = div().border_1().border_color(rgb(colors.border));
            let mut days = div().flex();
            for name in ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] {
                days = days.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .py_3()
                        .text_center()
                        .text_sm()
                        .bg(rgb(colors.subtle))
                        .child(name),
                );
            }
            grid = grid.child(days);
            for week in 0..6 {
                let mut row = div().flex();
                for weekday in 0..7 {
                    let day = first + chrono::Duration::days(week * 7 + weekday);
                    let selected = items_by_day.get(&day).map(Vec::as_slice).unwrap_or(&[]);
                    let mut cell = div()
                        .flex_1()
                        .min_w_0()
                        .h(px(138.))
                        .p_2()
                        .border_t_1()
                        .border_l_1()
                        .border_color(rgb(colors.border))
                        .overflow_hidden()
                        .bg(rgb(if day.month() == self.month.month() {
                            colors.surface
                        } else {
                            colors.subtle
                        }));
                    cell = cell.child(
                        div()
                            .id(SharedString::from(format!("calendar-day-{day}")))
                            .text_sm()
                            .cursor_pointer()
                            .text_color(rgb(if day == today {
                                colors.link
                            } else {
                                colors.muted
                            }))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(day.day().to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.calendar_day = Some(day);
                                cx.notify();
                            })),
                    );
                    for (index, item) in selected.iter().take(2).enumerate() {
                        let title = field(item, "title");
                        let mut event = div()
                            .id(SharedString::from(format!("calendar-event-{day}-{index}")))
                            .mt_2()
                            .px_1()
                            .text_xs()
                            .max_h(px(43.))
                            .overflow_hidden()
                            .cursor_pointer()
                            .text_color(rgb(colors
                                .category_text(model::course_color(&field(item, "course_id")))))
                            .child(title);
                        if let Some(route) = model::planner_route(item) {
                            event = event.on_click(cx.listener(move |this, _, window, cx| {
                                this.go(route.clone(), window, cx)
                            }));
                        } else if let Some(url) = item["html_url"].as_str() {
                            let url = url.to_owned();
                            if url::Url::parse(&url)
                                .is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
                            {
                                event = event.on_click(move |_, _, cx| cx.open_url(&url));
                            }
                        }
                        cell = cell.child(event);
                    }
                    if selected.len() > 2 {
                        cell = cell.child(muted(format!("+{} more", selected.len() - 2), cx));
                    }
                    row = row.child(cell);
                }
                grid = grid.child(row);
            }
            view = view.child(grid);
        }
        view.into_any_element()
    }
    fn course_picker(&self, which: Picker, cx: &mut Context<Self>) -> AnyElement {
        let mut picker = box_view(cx).gap_1();
        let empty_label = if which == Picker::NoteCourse {
            "Personal (no course)"
        } else {
            "All courses"
        };
        picker = picker.child(
            Button::new("course-picker-none")
                .label(empty_label)
                .small()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if which == Picker::NoteCourse {
                        if let Some(note) = &mut this.note {
                            note.course = None;
                            note.linked = None;
                        }
                    } else {
                        this.calendar_course = None;
                        this.load_route(cx);
                    }
                    this.picker = None;
                    cx.notify();
                })),
        );
        for course in self.courses() {
            let id = field(course, "id");
            picker = picker.child(
                Button::new(SharedString::from(format!("choose-course-{id}")))
                    .label(model::course_title(course))
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if which == Picker::NoteCourse {
                            if let Some(note) = &mut this.note {
                                note.course = Some(id.clone());
                                note.linked = None;
                            }
                            this.enqueue_read(
                                Request::new("assignment_list", json!({"course_id":id})),
                                false,
                            );
                            this.enqueue_read(
                                Request::new("module_list", json!({"course_id":id})),
                                false,
                            );
                            this.pump(cx);
                        } else {
                            this.calendar_course = Some(id.clone());
                            this.load_route(cx);
                        }
                        this.picker = None;
                        cx.notify();
                    })),
            );
        }
        picker.into_any_element()
    }
    fn account(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut view=div().max_w(px(760.)).flex().flex_col().gap_6().child(heading(if self.settings.origin.is_empty(){"Welcome to Canvas"}else{"Your Canvas account"}))
            .child(muted("Connect to your university’s Canvas website. Sign in using your usual institutional login — no API token is needed.", cx))
            .child(box_view(cx).child(div().font_weight(FontWeight::SEMIBOLD).child("Canvas website"))
                .child(Input::new(&self.origin)).child(muted("For example: https://canvas.university.edu", cx))
                .child(Button::new("save-campus").primary().label("Connect").disabled(self.demo||self.pending.is_some()).on_click(cx.listener(|this,_,_,cx|{
                    let settings=Settings{origin:this.origin.read(cx).value().to_string(),appearance:this.settings.appearance};if let Err(error)=settings.validate(){this.error=Some(error.to_string());cx.notify();return;}this.perform(Operation::Configure(settings),cx);
                }))))
            .child(box_view(cx).child(div().font_weight(FontWeight::SEMIBOLD).child("Sign in"))
                .child(muted("A browser window opens for your university login. Finish signing in and close that browser window to return here.", cx))
                .child(div().flex().gap_3()
                    .child(Button::new("sign-in-browser").primary().label("Sign in with browser").disabled(self.demo||self.settings.validate().is_err()||self.pending.is_some()).on_click(cx.listener(|this,_,_,cx|this.perform(Operation::Tool{name:"auth".into(),args:json!({})},cx))))
                    .child(Button::new("restore-session").label("Use saved login").disabled(self.demo||self.settings.validate().is_err()||self.pending.is_some()).on_click(cx.listener(|this,_,_,cx|this.perform(Operation::Tool{name:"refresh_auth".into(),args:json!({})},cx))))))
            .child(muted("Your login is stored in this app’s own browser profile. Course content is fetched directly from Canvas.", cx));
        let mut appearance_choices = div().flex().flex_wrap().gap_3();
        for (label, preference) in [
            ("System", Appearance::System),
            ("Light", Appearance::Light),
            ("Dark", Appearance::Dark),
        ] {
            appearance_choices = appearance_choices.child(
                Button::new(SharedString::from(format!("appearance-{label}")))
                    .label(label)
                    .small()
                    .when(self.settings.appearance == preference, |b| b.primary())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_appearance(preference, window, cx)
                    })),
            );
        }
        view = view.child(
            box_view(cx)
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Appearance"))
                .child(muted(
                    "Choose a theme, or follow your system’s appearance. Your choice is saved.",
                    cx,
                ))
                .child(appearance_choices),
        );
        view = view.child(
            Button::new("show-technical")
                .label(if self.show_technical {
                    "Hide troubleshooting details"
                } else {
                    "Troubleshooting details"
                })
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_technical = !this.show_technical;
                    cx.notify();
                })),
        );
        if self.show_technical {
            if let Some(renderer) = &self.renderer {
                view = view.child(
                    box_view(cx)
                        .child(muted(format!("Renderer: {}", renderer.device_name), cx))
                        .child(muted(
                            format!(
                                "Driver: {} · {}",
                                renderer.driver_name, renderer.driver_info
                            ),
                            cx,
                        ))
                        .child(muted(
                            if renderer.is_software_emulated {
                                "Software rendering (CPU) — performance will be reduced."
                            } else {
                                "Hardware GPU rendering"
                            },
                            cx,
                        )),
                );
            }
            view=view.child(box_view(cx).child(muted("Chrome/Chromium is required for sign-in. The app uses WebDriver to save a session, then makes Canvas REST requests directly. It does not require a separate server or executable.", cx))
            .child(muted(self.error.clone().unwrap_or_else(||"No connection error to report.".into()), cx)));
        }
        view.into_any_element()
    }
    fn note_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(note) = &self.note else {
            return div().into_any_element();
        };
        let course_label = note
            .course
            .as_deref()
            .map(|id| self.course_name(id))
            .unwrap_or_else(|| "Personal (no course)".into());
        let mut editor = box_view(cx)
            .bg(rgb(colors.notice))
            .gap_4()
            .child(heading(if note.id.is_some() {
                "Edit To-do"
            } else {
                "Add To-do"
            }))
            .child(muted("Title", cx))
            .child(Input::new(&note.title))
            .child(muted("Date", cx))
            .child(div().w(px(220.)).child(Input::new(&note.date)))
            .child(muted("Time (24-hour, optional)", cx))
            .child(div().w(px(220.)).child(Input::new(&note.time)))
            .child(muted("Details", cx))
            .child(Input::new(&note.details).h(px(96.)))
            .child(
                Button::new("note-course")
                    .label(course_label)
                    .icon(IconName::ChevronDown)
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picker = if this.picker == Some(Picker::NoteCourse) {
                            None
                        } else {
                            Some(Picker::NoteCourse)
                        };
                        cx.notify();
                    })),
            );
        if self.picker == Some(Picker::NoteCourse) {
            editor = editor.child(self.course_picker(Picker::NoteCourse, cx));
        }
        if note.id.is_none()
            && let Some(course) = note.course.as_ref()
        {
            editor = editor.child(
                Button::new("note-linked-item")
                    .label(
                        note.linked
                            .as_ref()
                            .map(|v| format!("Linked to: {}", field(v, "title")))
                            .unwrap_or_else(|| "Link to course item (optional)".into()),
                    )
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picker = if this.picker == Some(Picker::LinkedItem) {
                            None
                        } else {
                            Some(Picker::LinkedItem)
                        };
                        cx.notify();
                    })),
            );
            if self.picker == Some(Picker::LinkedItem) {
                let mut choices = box_view(cx).gap_2().child(
                    Button::new("unlink-item")
                        .label("No linked item")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(note) = &mut this.note {
                                note.linked = None;
                            }
                            this.picker = None;
                            cx.notify();
                        })),
                );
                let mut links = Vec::new();
                for item in model::list(
                    self.get("assignment_list", json!({"course_id":course})),
                    "assignments",
                ) {
                    links.push(json!({"id":item["id"],"kind":"assignment","title":item["name"]}));
                }
                for module in model::list(
                    self.get("module_list", json!({"course_id":course})),
                    "modules",
                ) {
                    for item in model::list(Some(module), "items") {
                        let kind = match field(item, "item_type").as_str() {
                            "Quiz" => "quiz",
                            "Discussion" => "discussion_topic",
                            "Page" => "wiki_page",
                            _ => continue,
                        };
                        if !item["content_id"].is_null() {
                            links.push(
                                json!({"id":item["content_id"],"kind":kind,"title":item["title"]}),
                            );
                        }
                    }
                }
                for (index, link) in links.into_iter().enumerate() {
                    choices = choices.child(
                        Button::new(SharedString::from(format!("linked-choice-{index}")))
                            .label(field(&link, "title"))
                            .small()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(note) = &mut this.note {
                                    note.linked = Some(link.clone());
                                }
                                this.picker = None;
                                cx.notify();
                            })),
                    );
                }
                editor = editor.child(choices);
            }
        }
        if self.confirm_save {
            editor = editor.child(muted(
                "Save this to-do to your Canvas account? This change will be sent once.",
                cx,
            ));
        }
        editor
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Button::new("save-note")
                            .primary()
                            .label(if self.confirm_save {
                                "Confirm Save"
                            } else {
                                "Save"
                            })
                            .disabled(self.demo || self.pending.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.save_note(cx))),
                    )
                    .child(
                        Button::new("cancel-note")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.note = None;
                                this.confirm_save = false;
                                this.picker = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for CanvasApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.document_jump_reset {
            let page = self.document_page.to_string();
            self.document_jump
                .update(cx, |input, cx| input.set_value(page, window, cx));
            self.document_jump_reset = false;
        }
        let colors = palette(cx);
        let title = self.page_title();
        let window_title = format!(
            "{title} — Canvas Native{}",
            if self.pending.is_some() {
                " (Loading…)"
            } else {
                ""
            }
        );
        crate::graphics::update_window_title(&mut self.window_title, window_title, |title| {
            window.set_window_title(title)
        });
        // A reader window is only the document: no global navigation, app
        // header or footer competes with the page area.
        if self.standalone_document
            && let Some(document) = self.active_document()
        {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .font_family("Inter")
                .text_base()
                .text_color(rgb(colors.text))
                .bg(rgb(colors.background))
                .child(self.document_viewer(&document, ReaderLayout::Window, window, cx))
                .into_any_element();
        }
        let reading_document = matches!(self.route, Route::Document { .. });
        let mut header = div()
            .flex()
            .items_center()
            .gap_4()
            .h(px(70.))
            .px_7()
            .border_b_1()
            .border_color(rgb(colors.border));
        if let Some(course) = self.route.course_id() {
            header = header
                .child(self.route_link(
                    "breadcrumb-course",
                    &self.course_name(course),
                    Route::Course {
                        id: course.into(),
                        tab: CourseTab::Home,
                    },
                    cx,
                ))
                .child(Icon::new(IconName::ChevronRight).text_color(rgb(colors.muted)));
        }
        header = header.child(div().text_xl().flex_1().min_w_0().child(title.clone()));
        if !matches!(self.route, Route::Account) {
            header = header.child(
                Button::new("refresh-page")
                    .label("Refresh")
                    .small()
                    .disabled(self.pending.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            );
        }
        let mut content = if reading_document {
            div().flex().flex_col().gap_1().p_1()
        } else {
            div().flex().flex_col().gap_6().p_7()
        };
        let virtual_modules = self.showing_modules();
        let virtual_document = self.showing_document_pages();
        if virtual_modules || virtual_document {
            content = content.flex_1().min_h_0().overflow_hidden();
        }
        if !matches!(
            self.route,
            Route::Account
                | Route::Assignment { .. }
                | Route::Page { .. }
                | Route::Document { .. }
                | Route::Note(_)
        ) {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .justify_between()
                    .child(heading(title))
                    .child(div().w(px(245.)).child(Input::new(&self.search))),
            );
        }
        if self.error.is_some() {
            content=content.child(div().p_4().bg(rgb(colors.warning)).border_1().border_color(rgb(colors.warning_border)).rounded_sm().flex().gap_4().items_center()
            .child(div().flex_1().text_sm().child(if self.show_technical{self.error.clone().unwrap_or_default()}else{"Canvas could not complete the last request. Try Refresh, or open Account to sign in again.".into()}))
            .child(self.route_button("error-account","Account",Route::Account,cx))
            .child(Button::new("dismiss-error").icon(IconName::Close).small().on_click(cx.listener(|this,_,_,cx|{this.error=None;cx.notify();}))));
        }
        if let Some(path) = &self.last_saved {
            let open = path.clone();
            let reveal = path.clone();
            content = content.child(
                box_view(cx).child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().flex_1().text_sm().child(format!(
                            "Downloaded {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        )))
                        .child(
                            Button::new("open-download")
                                .label("Open")
                                .small()
                                .on_click(move |_, _, cx| cx.open_with_system(&open)),
                        )
                        .child(
                            Button::new("reveal-download")
                                .label("Show in folder")
                                .small()
                                .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                        )
                        .child(
                            Button::new("dismiss-download")
                                .icon(IconName::Close)
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.last_saved = None;
                                    cx.notify();
                                })),
                        ),
                ),
            );
        }
        if self.note.is_some() {
            content = content.child(self.note_editor(cx));
        }
        if let Some(id) = self.confirm_delete.clone() {
            content =
                content.child(
                    box_view(cx)
                        .bg(rgb(colors.warning))
                        .child(heading("Delete this to-do?"))
                        .child(muted(
                            "This permanently removes it from your Canvas calendar.",
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .child(
                                    Button::new("confirm-delete")
                                        .primary()
                                        .label("Delete To-do")
                                        .disabled(self.demo || self.pending.is_some())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.perform(
                                                Operation::Tool {
                                                    name: "delete_planner_note".into(),
                                                    args: json!({"note_id":id}),
                                                },
                                                cx,
                                            )
                                        })),
                                )
                                .child(Button::new("keep-todo").label("Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.confirm_delete = None;
                                        cx.notify();
                                    }),
                                )),
                        ),
                );
        }
        let page = match self.route.clone() {
            Route::Dashboard => self.dashboard(cx),
            Route::Courses => self.all_courses(cx),
            Route::Calendar => self.calendar(cx),
            Route::Grades => self.grades(None, cx),
            Route::Account => self.account(cx),
            Route::Course { id, tab } => match tab {
                CourseTab::Home => self.course_home(&id, false, window, cx),
                CourseTab::Syllabus => self.course_home(&id, true, window, cx),
                CourseTab::Modules => self.modules(&id, cx),
                CourseTab::Assignments => self.assignments(&id, cx),
                CourseTab::Grades => self.grades(Some(&id), cx),
                CourseTab::Files => self.files(&id, cx),
            },
            Route::Assignment { course, id } => self.assignment_detail(&course, &id, window, cx),
            Route::Page { course, slug } => self.page_detail(&course, &slug, window, cx),
            Route::File { course, id } => self.file_detail(&course, &id, window, cx),
            Route::Document {
                course,
                resource,
                title,
                start,
            } => self.document(&course, &resource, &title, start, window, cx),
            Route::Note(id) => self.note_detail(&id, window, cx),
        };
        content = content.child(page);
        let mut body = div().flex().flex_1().min_h_0();
        if let Some(course) = self.route.course_id()
            && !reading_document
        {
            body = body.child(self.course_nav(course, cx));
        }
        body = body.child(
            div()
                .id("canvas-content")
                .flex_1()
                .min_w_0()
                .h_full()
                .when(!virtual_modules && !virtual_document, |d| {
                    d.overflow_y_scroll()
                })
                .when(virtual_modules, |d| d.flex().flex_col().overflow_hidden())
                .when(virtual_document, |d| d.flex().flex_col().overflow_hidden())
                .child(content),
        );
        let nav = if reading_document {
            None
        } else {
            Some(self.global_nav(cx))
        };
        let footer = if reading_document {
            None
        } else {
            Some(
                div()
                    .flex()
                    .gap_4()
                    .items_center()
                    .px_6()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(colors.border))
                    .text_xs()
                    .text_color(rgb(colors.muted))
                    .child(
                        Button::new("go-back")
                            .label("Back")
                            .small()
                            .disabled(self.history.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(div().flex_1().child(self.status.clone()))
                    .when(self.pending.is_some(), |d| {
                        d.child(
                            Button::new("cancel-loading")
                                .label("Cancel")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.queue.clear();
                                    this.prefetch_queue.clear();
                                    this.backend.cancel();
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.updated.is_some(), |d| {
                        d.child(format!("Updated {}", self.updated.as_deref().unwrap_or("")))
                    }),
            )
        };
        div()
            .size_full()
            .relative()
            .flex()
            .font_family("Inter")
            .text_base()
            .text_color(rgb(colors.text))
            .bg(rgb(colors.background))
            .children(nav)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .when(!reading_document, |d| d.child(header))
                    .child(body)
                    .children(footer),
            )
            .when(self.courses_open && !reading_document, |root| {
                root.child(
                    div()
                        .id("dismiss-courses-overlay")
                        .occlude()
                        .absolute()
                        .left(px(88.))
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .bg(gpui::black().opacity(0.2))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_courses(window, cx)),
                        ),
                )
                .child(self.courses_drawer(cx))
            })
            .into_any_element()
    }
}
fn item_date(item: &Value) -> Option<NaiveDate> {
    model::planner_date(item)
}
fn escape_text(value: &str) -> String {
    let mut text = String::new();
    for c in value.chars() {
        if "\\`*_{}[]<>()#+-.!|".contains(c) {
            text.push('\\');
        }
        text.push(c);
    }
    text
}

#[cfg(test)]
mod layout_tests {
    use super::module_row_container;
    use gpui::{Styled, relative};

    #[test]
    fn virtual_module_rows_explicitly_fill_the_list_width() {
        // Both header and item branches use this container. Flex growth alone
        // cannot stretch a row that List measures with layout_as_root.
        let mut row = module_row_container();
        assert_eq!(row.style().size.width, Some(relative(1.).into()));
    }
}
