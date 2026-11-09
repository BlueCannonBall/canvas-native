//! Scrolling regressions for the shared virtualized row list and the
//! virtualized text views. These assert that a screen with hundreds of rows
//! hands only a viewport-sized set of elements to GPUI and still scrolls.

#[cfg(test)]
mod tests {
    use crate::{
        app::CanvasApp,
        backend::Settings,
        model::{self, CourseTab, Route},
    };
    use gpui::{
        AppContext, Entity, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext,
        point, px,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;

    fn with_app(
        cx: &mut TestAppContext,
        run: impl FnOnce(Entity<CanvasApp>, &mut VisualTestContext),
    ) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("test runtime");
        let handle = runtime.handle().clone();
        std::mem::forget(runtime);
        let mut built = None;
        let (_app, cx) = cx.add_window_view(|window, cx| {
            gpui_component::init(cx);
            let app = cx.new(|cx| CanvasApp::new(Settings::default(), &handle, false, window, cx));
            built = Some(app.clone());
            gpui_component::Root::new(app, window, cx)
        });
        run(built.expect("app entity"), cx);
    }

    fn assignments(count: usize) -> Value {
        let items: Vec<_> = (0..count)
            .map(|index| {
                json!({
                    "id": format!("{index}"),
                    "course_id": "102",
                    "name": format!("Problem set {index} with a descriptively long title"),
                    "due_at": "2026-11-20T18:00:00Z",
                    "points_possible": 100,
                    "submission_types": ["online_upload"],
                    "submission": {"workflow_state": "unsubmitted", "missing": index % 3 == 0},
                })
            })
            .collect();
        json!({ "assignments": items })
    }

    fn courses(count: usize) -> Value {
        let items: Vec<_> = (0..count)
            .map(|index| {
                json!({
                    "id": format!("{index}"),
                    "name": format!("Course {index} with a long name"),
                    "course_code": format!("CRS {index}"),
                    "workflow_state": "available",
                    "term": {"name": "Term"},
                    "teachers": [{"display_name": "Teacher"}],
                })
            })
            .collect();
        json!({ "courses": items })
    }

    fn scroll_and_draw(cx: &mut VisualTestContext, app: &Entity<CanvasApp>) {
        // Aim at the blank padding at the bottom of the first row. That point
        // belongs to the list alone, so the wheel reaches the list's scroll
        // handler regardless of which interactive elements a row contains.
        let position = cx.update(|_, cx| {
            let state = &app.read(cx).content_scroll;
            let bounds = state
                .bounds_for_item(1)
                .or_else(|| state.bounds_for_item(0))
                .unwrap_or(state.viewport_bounds());
            point(bounds.left() + px(300.), bounds.bottom() - px(6.))
        });
        // The scroll handler consults the hit test from the last frame, so move
        // the pointer over the list and draw before delivering the wheel event.
        cx.simulate_event(gpui::MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Default::default(),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear();
        });
        // The first wheel event can only move within the rows measured so
        // far; drawing measures the next rows, then the second event reaches
        // the next item even when each row is tall.
        for _ in 0..2 {
            cx.simulate_event(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(-200.))),
                ..Default::default()
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear();
            });
        }
    }

    fn list_height(cx: &mut VisualTestContext, app: &Entity<CanvasApp>) -> f32 {
        cx.update(|_, cx| f32::from(app.read(cx).content_scroll.viewport_bounds().size.height))
    }

    fn list_scrolled(cx: &mut VisualTestContext, app: &Entity<CanvasApp>) -> bool {
        cx.update(|_, cx| {
            let top = app.read(cx).content_scroll.logical_scroll_top();
            top.item_ix > 0 || top.offset_in_item > px(0.)
        })
    }

    #[gpui::test]
    fn a_large_assignment_list_scrolls_without_building_every_row(cx: &mut TestAppContext) {
        with_app(cx, |app, cx| {
            cx.update(|window, cx| {
                app.update(cx, |app, _cx| {
                    app.route = Route::Course {
                        id: "102".into(),
                        tab: CourseTab::Assignments,
                    };
                    let key = model::cache_key("assignment_list", &json!({"course_id": "102"}));
                    app.data.insert(key, assignments(400));
                });
                window.draw(cx).clear();
            });
            assert_eq!(
                cx.update(|_, cx| app.read(cx).content_scroll.item_count()),
                401,
                "filters plus 400 assignments"
            );
            assert!(
                list_height(cx, &app) > 100.0,
                "assignment list must own the remaining height"
            );
            assert!(!list_scrolled(cx, &app));
            scroll_and_draw(cx, &app);
            assert!(
                list_scrolled(cx, &app),
                "the assignment list should scroll, not the page"
            );

            // A filter change replaces the prepared rows and returns to the top.
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.assignment_filter = "Missing".into();
                    cx.notify();
                });
                window.draw(cx).clear();
            });
            assert_eq!(
                cx.update(|_, cx| app.read(cx).content_scroll.item_count()),
                135,
                "filters and the 134 missing assignments"
            );
            assert!(!list_scrolled(cx, &app));
            scroll_and_draw(cx, &app);
            assert!(list_scrolled(cx, &app));
        });
    }

    #[gpui::test]
    fn grades_files_and_calendar_agenda_share_the_virtual_list(cx: &mut TestAppContext) {
        with_app(cx, |app, cx| {
            // Grades.
            cx.update(|window, cx| {
                app.update(cx, |app, _cx| {
                    app.route = Route::Course {
                        id: "102".into(),
                        tab: CourseTab::Grades,
                    };
                    let rows: Vec<_> = (0..400)
                        .map(|index| {
                            json!({"assignment_id": index.to_string(), "name": format!("Graded item {index} with a long title"), "score": 90, "points_possible": 100, "status": "graded", "due_at": "2026-11-20T18:00:00Z"})
                        })
                        .collect();
                    let key = model::cache_key("course_grades", &json!({"course_id": "102"}));
                    app.data.insert(
                        key,
                        json!({"course_grades": {"course_id": "102", "total_grade": {"current_score": 90.0, "current_grade": "A", "final_score": 88.0, "final_grade": "B+"}, "assignments": rows}}),
                    );
                });
                window.draw(cx).clear();
            });
            assert_eq!(
                cx.update(|_, cx| app.read(cx).content_scroll.item_count()),
                402,
                "summary, header, and 400 grades"
            );
            assert!(list_height(cx, &app) > 100.0);
            scroll_and_draw(cx, &app);
            assert!(list_scrolled(cx, &app), "grades should scroll");

            // Files.
            cx.update(|window, cx| {
                app.update(cx, |app, _cx| {
                    app.route = Route::Course {
                        id: "102".into(),
                        tab: CourseTab::Files,
                    };
                    let items: Vec<_> = (0..400)
                        .map(|index| {
                            json!({"title": format!("File {index} with a long descriptive name.pdf"), "item_type": "File", "content_id": index.to_string()})
                        })
                        .collect();
                    let key = model::cache_key("module_list", &json!({"course_id": "102"}));
                    app.data.insert(
                        key,
                        json!({"modules": [{"id": "1", "name": "Module", "items": items}]}),
                    );
                });
                window.draw(cx).clear();
            });
            assert_eq!(
                cx.update(|_, cx| app.read(cx).content_scroll.item_count()),
                402,
                "intro, one module heading, and 400 files"
            );
            assert!(list_height(cx, &app) > 100.0);
            scroll_and_draw(cx, &app);
            assert!(list_scrolled(cx, &app), "files should scroll");

            // Calendar agenda.
            cx.update(|window, cx| {
                app.update(cx, |app, _cx| {
                    app.route = Route::Calendar;
                    app.calendar_list = true;
                    let request = app.planner_request(false);
                    let items: Vec<_> = (0..400)
                        .map(|index| {
                            json!({"item_id": index.to_string(), "item_type": "assignment", "course_id": "102", "title": format!("Agenda item {index} with a long title"), "due_at": "2026-11-20T18:00:00Z"})
                        })
                        .collect();
                    let key = model::cache_key(&request.name, &request.args);
                    app.data.insert(key, json!({"items": items}));
                });
                window.draw(cx).clear();
            });
            assert_eq!(
                cx.update(|_, cx| app.read(cx).content_scroll.item_count()),
                403,
                "controls, filters, range, and 400 items"
            );
            assert!(list_height(cx, &app) > 100.0);
            scroll_and_draw(cx, &app);
            assert!(list_scrolled(cx, &app), "agenda should scroll");
        });
    }

    #[gpui::test]
    fn html_body_is_prepared_once_per_page_revision(cx: &mut TestAppContext) {
        with_app(cx, |app, cx| {
            let html: String = (0..150)
                .map(|index| {
                    format!(
                        "<h3>Section {index}</h3><p>Paragraph {index} with enough text to wrap.</p>"
                    )
                })
                .collect();
            cx.update(|window, cx| {
                app.update(cx, |app, _cx| {
                    app.route = Route::Assignment {
                        course: "102".into(),
                        id: "202".into(),
                    };
                    let key = model::cache_key(
                        "assignment_info",
                        &json!({"course_id": "102", "assignment_id": "202"}),
                    );
                    app.data.insert(
                        key,
                        json!({"assignment": {"id": "202", "name": "Problem set", "due_at": "2026-11-20T18:00:00Z", "points_possible": 100, "description_html": html, "submission": {"workflow_state": "unsubmitted"}}}),
                    );
                });
                window.draw(cx).clear();
            });
            let first = cx.update(|_, cx| {
                app.read(cx)
                    .prepared_body
                    .borrow()
                    .as_ref()
                    .map(|(_, _, body)| body.clone())
                    .expect("prepared body")
            });
            cx.update(|window, cx| {
                window.draw(cx).clear();
            });
            let second = cx.update(|_, cx| {
                app.read(cx)
                    .prepared_body
                    .borrow()
                    .as_ref()
                    .map(|(_, _, body)| body.clone())
                    .expect("prepared body")
            });
            assert!(
                Arc::ptr_eq(&first, &second),
                "the same HTML body must not be parsed again on each scroll frame"
            );
        });
    }

    #[gpui::test]
    fn wheel_over_the_pinned_search_scrolls_the_content_below(cx: &mut TestAppContext) {
        with_app(cx, |app, cx| {
            cx.update(|window, cx| {
                app.update(cx, |app, _cx| {
                    app.route = Route::Course {
                        id: "102".into(),
                        tab: CourseTab::Assignments,
                    };
                    let key = model::cache_key("assignment_list", &json!({"course_id": "102"}));
                    app.data.insert(key, assignments(400));
                });
                window.draw(cx).clear();
            });
            let heading = cx
                .debug_bounds("page-heading-bounds")
                .expect("pinned page heading");
            let position = heading.center();
            cx.simulate_event(gpui::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            });
            cx.update(|window, cx| {
                window.draw(cx).clear();
            });
            cx.simulate_event(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(-400.))),
                ..Default::default()
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear();
            });
            assert!(
                list_scrolled(cx, &app),
                "the pinned search area must scroll the list below it"
            );

            // The same forwarding applies to ordinary page scrolling.
            cx.update(|window, cx| {
                app.update(cx, |app, _cx| {
                    app.route = Route::Courses;
                    let key = model::cache_key("course_list", &json!({}));
                    app.data.insert(key, courses(60));
                });
                window.draw(cx).clear();
            });
            let heading = cx
                .debug_bounds("page-heading-bounds")
                .expect("pinned page heading");
            let position = heading.center();
            cx.simulate_event(gpui::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            });
            cx.update(|window, cx| {
                window.draw(cx).clear();
            });
            cx.simulate_event(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(-400.))),
                ..Default::default()
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.draw(cx).clear();
            });
            assert!(
                cx.update(|_, cx| app.read(cx).page_scroll.offset().y) < px(0.),
                "the pinned search area must scroll the page below it"
            );
        });
    }
}
