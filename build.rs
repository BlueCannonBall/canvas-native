//! Embeds the application icon into Windows executables.
//!
//! GPUI's Windows backend loads its window icon from resource ID 1 in the
//! running executable, and Explorer and the taskbar use the same resource.
//! Linux keeps its X11 property and desktop-entry icon in `window_icon.rs`;
//! this build step does nothing for other target operating systems.

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/canvas-native.rc");
    println!("cargo:rerun-if-changed=assets/icons/edu.canvasnative.CanvasNative.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/icons/canvas-native.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("could not embed the Windows application icon");
    }
}
