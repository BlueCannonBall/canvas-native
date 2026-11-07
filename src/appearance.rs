use gpui::{App, Window};
use gpui_component::{ActiveTheme, Theme, ThemeMode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}
impl Appearance {
    pub fn apply(self, window: Option<&mut Window>, cx: &mut App) {
        match self {
            Self::System => Theme::sync_system_appearance(window, cx),
            Self::Light => Theme::change(ThemeMode::Light, window, cx),
            Self::Dark => Theme::change(ThemeMode::Dark, window, cx),
        }
    }
}
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub text: u32,
    pub muted: u32,
    pub link: u32,
    pub border: u32,
    pub background: u32,
    pub surface: u32,
    pub subtle: u32,
    pub nav: u32,
    pub nav_text: u32,
    pub hover: u32,
    pub nav_hover: u32,
    pub warning: u32,
    pub warning_border: u32,
    pub notice: u32,
    pub success: u32,
    pub missing: u32,
    pub complete: u32,
}
impl Palette {
    pub fn for_dark(dark: bool) -> Self {
        if dark {
            Self {
                dark: true,
                text: 0xe2e9ef,
                muted: 0xabb8c2,
                link: 0x78cdf8,
                border: 0x414d58,
                background: 0x181f26,
                surface: 0x202932,
                subtle: 0x2a3540,
                nav: 0x101a23,
                nav_text: 0xe9f0f5,
                hover: 0x2e3b47,
                nav_hover: 0x2a3a48,
                warning: 0x3e2921,
                warning_border: 0x805742,
                notice: 0x233544,
                success: 0x77d3a8,
                missing: 0x4a3025,
                complete: 0x204334,
            }
        } else {
            Self {
                dark: false,
                text: 0x2d3b45,
                muted: 0x606f78,
                link: 0x0875a3,
                border: 0xd7dce0,
                background: 0xffffff,
                surface: 0xffffff,
                subtle: 0xf1f3f4,
                nav: 0x273540,
                nav_text: 0xffffff,
                hover: 0xf1f5f7,
                nav_hover: 0x3c4b56,
                warning: 0xfff3ee,
                warning_border: 0xf0c8ba,
                notice: 0xf5f9fb,
                success: 0x25836b,
                missing: 0xffeee5,
                complete: 0xeaf5ef,
            }
        }
    }
    pub fn category_text(self, base: u32) -> u32 {
        if !self.dark {
            return base;
        }
        let channel = |shift: u32| (((base >> shift) & 255u32) + 255u32) / 2u32;
        (channel(16) << 16) | (channel(8) << 8) | channel(0)
    }
}
pub fn palette(cx: &App) -> Palette {
    Palette::for_dark(cx.theme().mode.is_dark())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let value = ((color >> shift) & 255u32) as f64 / 255.;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }
    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn both_palettes_keep_readable_text_and_links() {
        for dark in [false, true] {
            let p = Palette::for_dark(dark);
            for foreground in [p.text, p.muted, p.link] {
                for background in [p.surface, p.background, p.subtle, p.warning, p.notice] {
                    assert!(
                        contrast(foreground, background) >= 4.5,
                        "Insufficient contrast in dark={dark}: {foreground:x}"
                    );
                }
            }
        }
    }
    #[test]
    fn appearance_roundtrips_and_defaults_to_system() {
        assert_eq!(
            serde_json::to_string(&Appearance::Dark).unwrap(),
            "\"dark\""
        );
        assert_eq!(Appearance::default(), Appearance::System);
    }
    #[test]
    fn course_color_labels_remain_readable_in_dark_calendar() {
        let p = Palette::for_dark(true);
        for course in ["101", "102", "103", "104", "105", "106"] {
            assert!(
                contrast(
                    p.category_text(crate::model::course_color(course)),
                    p.surface
                ) >= 4.5
            );
        }
    }
}
