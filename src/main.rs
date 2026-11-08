#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod appearance;
mod backend;
mod demo;
mod document_preview;
mod engine;
mod graphics;
mod html_content;
mod model;
mod module_list;
mod native_results;
mod schema;
mod window_icon;
#[cfg(target_os = "windows")]
mod windows_browser;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions,
    actions, px, size,
};
use gpui_component::Root;

actions!(canvas_native, [Quit, CloseCourses]);

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--install-desktop") {
        return window_icon::install_desktop();
    }
    if std::env::args().any(|a| a == "--renderer-info") {
        return graphics::print_renderer_info();
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let settings = backend::Settings::load();
    if std::env::args().any(|a| a == "--check-course-homes") {
        settings.validate()?;
        runtime.block_on(async {
            let engine = engine::Engine::new(settings.origin.trim())?;
            let courses =
                schema::payload(&engine.call("course_list", serde_json::json!({})).await?);
            for course in model::list(Some(&courses), "courses") {
                let id = model::field(course, "id");
                let info = schema::payload(
                    &engine
                        .call("course_info", serde_json::json!({"course_id":id}))
                        .await?,
                );
                let home = model::course_home_view(&info["course"]);
                println!("{}: {home}", model::course_title(&info["course"]));
                if home == "wiki" {
                    let front = schema::payload(
                        &engine
                            .call("course_front_page", serde_json::json!({"course_id":id}))
                            .await?,
                    );
                    println!(
                        "  Front page: {} ({} HTML bytes)",
                        model::field(&front["page"], "title"),
                        model::field(&front["page"], "body_html").len()
                    );
                }
            }
            Ok::<_, anyhow::Error>(())
        })?;
        return Ok(());
    }
    if std::env::args().any(|a| a == "--check-backend") {
        let backend = backend::Backend::start(settings, runtime.handle());
        let tools = runtime.block_on(backend.request(backend::Operation::Discover))??;
        let tools = tools
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid operation catalog"))?;
        println!(
            "{} in-process Canvas operations available without a connector executable:",
            tools.len()
        );
        for tool in tools {
            println!("  {}", tool["name"].as_str().unwrap_or("unknown"));
        }
        drop(backend);
        runtime.shutdown_timeout(std::time::Duration::from_secs(6));
        return Ok(());
    }
    let demo = std::env::args().any(|a| a == "--demo");
    let handle = runtime.handle().clone();
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            settings.appearance.apply(None, cx);
            cx.on_action(|_: &Quit, cx| request_quit(cx));
            cx.bind_keys([
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("escape", CloseCourses, Some("CoursesDrawer")),
            ]);
            // Linux GPUI stops its event loop when the last native window is
            // dropped. Calling quit from on_window_closed re-enters the X11
            // client's RefCell while WM_DELETE_WINDOW still borrows it.
            #[cfg(not(target_os = "linux"))]
            cx.on_window_closed(|cx| {
                cx.spawn(async move |cx| {
                    let _ = cx.update(|cx| {
                        if cx.windows().is_empty() {
                            cx.quit();
                        }
                    });
                })
                .detach();
            })
            .detach();
            let (width, height) = cx
                .primary_display()
                .map(|display| {
                    let bounds = display.bounds();
                    (
                        (f32::from(bounds.size.width) * 0.9).clamp(760., 1500.),
                        (f32::from(bounds.size.height) * 0.9).clamp(480., 900.),
                    )
                })
                .unwrap_or((1100., 760.));
            let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
            cx.open_window(
                WindowOptions {
                    app_id: Some(window_icon::APP_ID.into()),
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(760.), px(480.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Canvas Native".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    if let Err(error) = window_icon::set_window_icon() {
                        eprintln!("Window icon unavailable: {error}");
                    }
                    if let Some(info) = window.gpu_specs() {
                        eprintln!(
                            "Canvas renderer: {} / {} (software emulation: {})",
                            info.device_name, info.driver_name, info.is_software_emulated
                        );
                    }
                    let view =
                        cx.new(|cx| app::CanvasApp::new(settings, &handle, demo, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Could not open the Canvas window");
            cx.activate(true);
        });
    runtime.shutdown_timeout(std::time::Duration::from_secs(6));
    Ok(())
}

fn request_quit(cx: &mut App) {
    // A foreground task runs after the native event callback returns. App::defer
    // is not sufficient here: it can run before that callback releases its borrow.
    cx.spawn(async move |cx| {
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}
