//! Window-manager icon and matching Linux desktop identity; no display input.
pub const APP_ID: &str = "edu.canvasnative.CanvasNative";
const ICON_SVG: &[u8] =
    include_bytes!("../assets/icons/edu.canvasnative.CanvasNative-chalkboard.svg");
const SYMBOLIC_SVG: &[u8] =
    include_bytes!("../assets/icons/edu.canvasnative.CanvasNative-symbolic.svg");

fn icon_pixels(size: u32) -> anyhow::Result<Vec<u32>> {
    let tree = usvg::Tree::from_data(ICON_SVG, &usvg::Options::default())?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)
        .ok_or_else(|| anyhow::anyhow!("Invalid icon dimensions: {size}"))?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        size as f32 / tree.size().width(),
        size as f32 / tree.size().height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia stores premultiplied RGBA; both PNG and _NET_WM_ICON need
    // straight-alpha channels. Demultiply before converting to ARGB words.
    Ok(pixmap
        .pixels()
        .iter()
        .map(|pixel| {
            let color = pixel.demultiply();
            u32::from_be_bytes([color.alpha(), color.red(), color.green(), color.blue()])
        })
        .collect())
}

pub fn set_window_icon() -> anyhow::Result<()> {
    // GPUI 0.2.2's X11Window::window_handle() is unimplemented and panics.
    // Never call it, even to ask which platform this window belongs to.
    if std::env::var_os("DISPLAY").is_none() {
        return Ok(());
    }
    let (connection, _) = x11rb::connect(None)?;
    set_owned_x11_icons(&connection, std::process::id())?;
    // Wayland uses app_id to find the matching desktop entry/icon.
    Ok(())
}

fn owned_window(pid: Option<u32>, class: &[u8], current_pid: u32) -> bool {
    pid == Some(current_pid) && class.split(|b| *b == 0).any(|s| s == APP_ID.as_bytes())
}

fn set_owned_x11_icons(
    connection: &impl x11rb::connection::Connection,
    pid: u32,
) -> anyhow::Result<usize> {
    use x11rb::{
        protocol::xproto::{AtomEnum, ConnectionExt, PropMode},
        wrapper::ConnectionExt as _,
    };
    let pid_atom = connection.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
    let icon_atom = connection
        .intern_atom(false, b"_NET_WM_ICON")?
        .reply()?
        .atom;
    let mut data = Vec::new();
    for size in [32, 64, 128] {
        data.extend([size, size]);
        data.extend(icon_pixels(size)?);
    }
    let mut updated = 0;
    // A window manager may already have reparented the new window into a frame.
    // Bound discovery and require BOTH process and class before any write.
    // No title matching, focus changes, synthetic input, or GPUI raw handles.
    for screen in &connection.setup().roots {
        let mut pending: std::collections::VecDeque<_> = connection
            .query_tree(screen.root)?
            .reply()?
            .children
            .into_iter()
            .map(|w| (w, 0))
            .collect();
        let mut inspected = 0;
        while let Some((window, depth)) = pending.pop_front() {
            inspected += 1;
            if inspected > 4096 {
                break;
            }
            let Ok(property) = connection
                .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)?
                .reply()
            else {
                continue;
            };
            let owner = property.value32().and_then(|mut values| values.next());
            if owner != Some(pid) {
                if depth < 2
                    && let Ok(tree) = connection.query_tree(window)?.reply()
                {
                    pending.extend(tree.children.into_iter().map(|w| (w, depth + 1)));
                }
                continue;
            }
            let Ok(class) = connection
                .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)?
                .reply()
            else {
                continue;
            };
            if !owned_window(owner, &class.value, pid) {
                continue;
            }
            connection
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    icon_atom,
                    AtomEnum::CARDINAL,
                    &data,
                )?
                .check()?;
            updated += 1;
        }
    }
    connection.flush()?;
    Ok(updated)
}

pub fn install_desktop() -> anyhow::Result<()> {
    let base =
        directories::BaseDirs::new().ok_or_else(|| anyhow::anyhow!("No user data directory"))?;
    install_desktop_at(base.data_dir(), &std::env::current_exe()?)
}

fn install_desktop_at(
    data_dir: &std::path::Path,
    executable: &std::path::Path,
) -> anyhow::Result<()> {
    use std::io::Write;
    let icons = data_dir.join("icons/hicolor/256x256/apps");
    let scalable = data_dir.join("icons/hicolor/scalable/apps");
    let symbolic = data_dir.join("icons/hicolor/symbolic/apps");
    let applications = data_dir.join("applications");
    let icon_path = icons.join(format!("{APP_ID}.png"));
    let svg_path = scalable.join(format!("{APP_ID}.svg"));
    let symbolic_path = symbolic.join(format!("{APP_ID}-symbolic.svg"));
    let desktop_path = applications.join(format!("{APP_ID}.desktop"));
    anyhow::ensure!(
        [&icon_path, &svg_path, &symbolic_path, &desktop_path]
            .iter()
            .all(|path| !path.exists()),
        "Desktop integration already exists; leaving existing files unchanged."
    );
    std::fs::create_dir_all(icons)?;
    std::fs::create_dir_all(scalable)?;
    std::fs::create_dir_all(symbolic)?;
    std::fs::create_dir_all(applications)?;
    let bytes: Vec<u8> = icon_pixels(256)?
        .into_iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8])
        .collect();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 256, 256);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&bytes)?;
    }
    let executable = executable
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&icon_path)?
        .write_all(&png)?;
    for (path, bytes) in [(&svg_path, ICON_SVG), (&symbolic_path, SYMBOLIC_SVG)] {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(bytes)?;
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(&desktop_path)?.write_all(
        format!(
            "[Desktop Entry]\nType=Application\nName=Canvas Native\nComment=Native Canvas LMS client\nExec=\"{executable}\"\nIcon={APP_ID}\nStartupWMClass={APP_ID}\nTerminal=false\nCategories=Education;\n"
        ).as_bytes(),
    )?;
    println!(
        "Installed {}\nInstalled {}",
        icon_path.display(),
        desktop_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_app_identity_and_current_process_can_receive_an_icon() {
        let class = format!("{APP_ID}\0{APP_ID}\0");
        assert!(owned_window(Some(42), class.as_bytes(), 42));
        assert!(!owned_window(Some(41), class.as_bytes(), 42));
        assert!(!owned_window(None, class.as_bytes(), 42));
        assert!(!owned_window(Some(42), b"other-app\0other-app\0", 42));
        assert!(!owned_window(
            Some(42),
            format!("{APP_ID}.other\0").as_bytes(),
            42
        ));
    }
    #[test]
    #[ignore = "requires an explicitly supplied isolated X11 server, never shared :0"]
    fn writes_icon_to_owned_window_without_using_gpui_raw_handles() {
        use x11rb::{
            connection::Connection,
            protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass},
            wrapper::ConnectionExt as _,
        };
        let display = std::env::var("CANVAS_NATIVE_X11_TEST_DISPLAY")
            .expect("Supply an isolated X11 display");
        assert!(
            display.starts_with(':') && display.split('.').next() != Some(":0"),
            "Never use shared :0"
        );
        let (connection, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = connection.setup().roots[screen].root;
        let pid_atom = connection
            .intern_atom(false, b"_NET_WM_PID")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let icon_atom = connection
            .intern_atom(false, b"_NET_WM_ICON")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let class = format!("{APP_ID}\0{APP_ID}\0");
        let mut windows = Vec::new();
        for (pid, class) in [
            (42, class.as_bytes()),
            (43, class.as_bytes()),
            (42, b"other-app\0other-app\0".as_slice()),
        ] {
            let window = connection.generate_id().unwrap();
            connection
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    window,
                    root,
                    0,
                    0,
                    64,
                    64,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    0,
                    &CreateWindowAux::new(),
                )
                .unwrap()
                .check()
                .unwrap();
            connection
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    pid_atom,
                    AtomEnum::CARDINAL,
                    &[pid],
                )
                .unwrap()
                .check()
                .unwrap();
            connection
                .change_property8(
                    PropMode::REPLACE,
                    window,
                    AtomEnum::WM_CLASS,
                    AtomEnum::STRING,
                    class,
                )
                .unwrap()
                .check()
                .unwrap();
            windows.push(window);
        }
        assert_eq!(set_owned_x11_icons(&connection, 42).unwrap(), 1);
        let frame = connection.generate_id().unwrap();
        connection
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                frame,
                root,
                0,
                0,
                64,
                64,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        connection
            .reparent_window(windows[0], frame, 0, 0)
            .unwrap()
            .check()
            .unwrap();
        connection
            .delete_property(windows[0], icon_atom)
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(set_owned_x11_icons(&connection, 42).unwrap(), 1);
        for (index, window) in windows.into_iter().enumerate() {
            let property = connection
                .get_property(false, window, icon_atom, AtomEnum::CARDINAL, 0, u32::MAX)
                .unwrap()
                .reply()
                .unwrap();
            if index == 0 {
                let data: Vec<_> = property.value32().unwrap().collect();
                assert_eq!(&data[..2], &[32, 32]);
                assert_eq!(data.len(), 6 + 32 * 32 + 64 * 64 + 128 * 128);
                let mut offset = 0;
                for size in [32, 64, 128] {
                    assert_eq!(&data[offset..offset + 2], &[size, size]);
                    offset += 2;
                    let end = offset + (size * size) as usize;
                    assert_eq!(&data[offset..end], icon_pixels(size).unwrap());
                    offset = end;
                }
            } else {
                assert!(property.value.is_empty());
            }
            connection.destroy_window(window).unwrap().check().unwrap();
        }
        connection.destroy_window(frame).unwrap().check().unwrap();
    }
    #[test]
    fn chalkboard_renders_at_all_window_and_launcher_sizes() {
        for size in [32, 64, 128, 256] {
            let pixels = icon_pixels(size).unwrap();
            assert_eq!(pixels.len(), (size * size) as usize);
            assert_eq!(pixels[0], 0);
            assert_eq!(pixels[((size / 2) * size + size / 2) as usize] >> 24, 255);
            // Frame, green board and white chalk must all survive scaling.
            assert!(pixels.iter().any(|p| (p >> 24) == 255
                && ((p >> 16) & 255) > 160
                && ((p >> 8) & 255) > 120
                && (p & 255) < 180));
            assert!(pixels.iter().any(|p| (p >> 24) == 255
                && ((p >> 8) & 255) > ((p >> 16) & 255)
                && ((p >> 8) & 255) < 120));
            assert!(pixels.iter().any(|p| (p >> 24) == 255 && (p & 255) > 220));
        }
    }

    #[test]
    fn desktop_install_uses_embedded_chalkboard_and_preserves_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        install_desktop_at(dir.path(), std::path::Path::new("/test/Canvas Native")).unwrap();
        let icons = dir.path().join("icons/hicolor");
        assert_eq!(
            std::fs::read(icons.join(format!("scalable/apps/{APP_ID}.svg"))).unwrap(),
            ICON_SVG
        );
        assert_eq!(
            std::fs::read(icons.join(format!("symbolic/apps/{APP_ID}-symbolic.svg"))).unwrap(),
            SYMBOLIC_SVG
        );
        let png = std::fs::read(icons.join(format!("256x256/apps/{APP_ID}.png"))).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        let expected: Vec<u8> = icon_pixels(256)
            .unwrap()
            .into_iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8])
            .collect();
        assert_eq!(decoded.dimensions(), (256, 256));
        assert_eq!(decoded.as_raw(), &expected);
        let desktop_path = dir.path().join(format!("applications/{APP_ID}.desktop"));
        let desktop = std::fs::read_to_string(&desktop_path).unwrap();
        assert!(desktop.contains(&format!("Icon={APP_ID}\n")));
        assert!(desktop.contains("Exec=\"/test/Canvas Native\""));
        std::fs::write(&desktop_path, "user-customized entry").unwrap();
        assert!(install_desktop_at(dir.path(), std::path::Path::new("/other")).is_err());
        assert_eq!(
            std::fs::read_to_string(desktop_path).unwrap(),
            "user-customized entry"
        );
    }
}
