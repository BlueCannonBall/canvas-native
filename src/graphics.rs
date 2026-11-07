//! Probe the same Blade/Vulkan device selection as GPUI, without creating a
//! window or sending any input to a display.
pub fn print_renderer_info() -> anyhow::Result<()> {
    let device_id = match std::env::var("ZED_DEVICE_ID") {
        Ok(value) => parse_device_id(&value)?,
        Err(std::env::VarError::NotPresent) => 0,
        Err(error) => return Err(error.into()),
    };
    // Matches GPUI's BladeContext::new configuration. No surface is created.
    let context = unsafe {
        blade_graphics::Context::init(blade_graphics::ContextDesc {
            presentation: true,
            device_id,
            ..Default::default()
        })
    }
    .map_err(|error| anyhow::anyhow!("Cannot initialize the GPUI graphics device: {error:?}"))?;
    let info = context.device_information();
    println!("Device: {}", info.device_name);
    println!("Driver: {} ({})", info.driver_name, info.driver_info);
    println!("Software emulation: {}", info.is_software_emulated);
    println!(
        "This probes device selection only; the window's actual renderer is reported at startup."
    );
    if info.is_software_emulated {
        println!("Warning: GPUI selected a CPU renderer. Check Vulkan driver/device overrides.");
    }
    Ok(())
}

fn parse_device_id(value: &str) -> anyhow::Result<u32> {
    let value = value.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    anyhow::ensure!(
        value.len() == 4 && value.chars().all(|c| c.is_ascii_hexdigit()),
        "ZED_DEVICE_ID must be a four-digit hexadecimal PCI device ID"
    );
    Ok(u32::from_str_radix(value, 16)?)
}

/// Avoid synchronous X11 property writes on unchanged render frames.
pub fn update_window_title(previous: &mut String, title: String, write: impl FnOnce(&str)) {
    if *previous != title {
        write(&title);
        *previous = title;
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_device_id, update_window_title};
    #[test]
    fn matches_gpui_device_override_syntax() {
        for value in ["1506", "0x1506", "0X1506", " 1506 "] {
            assert_eq!(parse_device_id(value).unwrap(), 0x1506);
        }
        for value in ["", "150", "0x15060", "llvmpipe", "zzzz"] {
            assert!(parse_device_id(value).is_err());
        }
    }

    #[test]
    fn unchanged_render_frames_do_not_write_window_properties() {
        let mut previous = String::new();
        let mut writes = Vec::new();
        for _ in 0..1000 {
            update_window_title(
                &mut previous,
                "Dashboard — Canvas Native".into(),
                |title| writes.push(title.to_owned()),
            );
        }
        update_window_title(
            &mut previous,
            "Assignments — Canvas Native".into(),
            |title| writes.push(title.to_owned()),
        );
        assert_eq!(
            writes,
            ["Dashboard — Canvas Native", "Assignments — Canvas Native"]
        );
    }
}
