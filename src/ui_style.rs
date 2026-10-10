//! Shared native widgets and semantic colors. Themes and tiny icons are cached.
use iced::{
    Color, Theme, color,
    widget::{button, container, image, text_input},
};
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub enum Tone {
    Accent,
    Success,
    Warning,
    Danger,
    Muted,
}

pub fn theme(dark: bool) -> Theme {
    static THEMES: OnceLock<[Theme; 2]> = OnceLock::new();
    THEMES.get_or_init(|| {
        [false, true].map(|dark| {
            Theme::custom(
                if dark { "Clash Dark" } else { "Clash Light" }.into(),
                iced::theme::Palette {
                    background: if dark {
                        color!(0x111827)
                    } else {
                        color!(0xF6F8FB)
                    },
                    text: if dark {
                        color!(0xF1F5F9)
                    } else {
                        color!(0x0F172A)
                    },
                    primary: color!(0x0F766E),
                    success: if dark {
                        color!(0x86EFAC)
                    } else {
                        color!(0x166534)
                    },
                    warning: if dark {
                        color!(0xFCD34D)
                    } else {
                        color!(0x92400E)
                    },
                    danger: if dark {
                        color!(0xFCA5A5)
                    } else {
                        color!(0xB91C1C)
                    },
                },
            )
        })
    })[usize::from(dark)]
    .clone()
}

pub fn dark(theme: &Theme) -> bool {
    theme.palette().background.r < 0.2
}
pub fn surface(theme: &Theme) -> Color {
    if dark(theme) {
        color!(0x1E293B)
    } else {
        Color::WHITE
    }
}
pub fn secondary(theme: &Theme) -> Color {
    if dark(theme) {
        color!(0xCBD5E1)
    } else {
        color!(0x475569)
    }
}
pub fn border(theme: &Theme) -> Color {
    if dark(theme) {
        color!(0x475569)
    } else {
        color!(0xCBD5E1)
    }
}
pub fn tone(theme: &Theme, tone: Tone) -> Color {
    match tone {
        Tone::Accent => {
            if dark(theme) {
                color!(0x5EEAD4)
            } else {
                color!(0x0F766E)
            }
        }
        Tone::Success => theme.palette().success,
        Tone::Warning => theme.palette().warning,
        Tone::Danger => theme.palette().danger,
        Tone::Muted => secondary(theme),
    }
}
fn mix(a: Color, b: Color, fraction: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * fraction,
        a.g + (b.g - a.g) * fraction,
        a.b + (b.b - a.b) * fraction,
    )
}
pub fn panel(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(surface(theme).into()),
        text_color: Some(theme.palette().text),
        border: iced::Border {
            color: border(theme),
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}
pub fn selected_panel(theme: &Theme) -> container::Style {
    badge(theme, Tone::Accent)
}
pub fn badge(theme: &Theme, kind: Tone) -> container::Style {
    let ink = tone(theme, kind);
    container::Style {
        background: Some(mix(surface(theme), ink, if dark(theme) { 0.09 } else { 0.07 }).into()),
        text_color: Some(ink),
        border: iced::Border {
            color: mix(border(theme), ink, 0.45),
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}
pub fn table_row(theme: &Theme, alternate: bool) -> container::Style {
    let mut style = panel(theme);
    style.border.radius = 0.0.into();
    style.border.width = 0.0;
    if alternate {
        style.background = Some(mix(surface(theme), theme.palette().background, 0.6).into());
    }
    style
}
pub fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(
            if dark(theme) {
                color!(0x172033)
            } else {
                color!(0xEDF2F7)
            }
            .into(),
        ),
        text_color: Some(theme.palette().text),
        ..Default::default()
    }
}
pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.text_color = Color::WHITE;
    style.background = Some(
        match status {
            button::Status::Hovered => color!(0x115E59),
            button::Status::Pressed => color!(0x134E4A),
            button::Status::Disabled => mix(surface(theme), theme.palette().text, 0.12),
            button::Status::Active => color!(0x0F766E),
        }
        .into(),
    );
    if matches!(status, button::Status::Disabled) {
        style.text_color = secondary(theme);
    }
    style.border.radius = 6.0.into();
    style
}
pub fn secondary_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::secondary(theme, status);
    style.background = Some(
        if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            mix(surface(theme), tone(theme, Tone::Accent), 0.09)
        } else {
            surface(theme)
        }
        .into(),
    );
    style.text_color = if matches!(status, button::Status::Disabled) {
        secondary(theme)
    } else {
        theme.palette().text
    };
    style.border = iced::Border {
        color: border(theme),
        width: 1.0,
        radius: 6.0.into(),
    };
    style
}
pub fn text_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = secondary_button(theme, status);
    style.border.width = 0.0;
    if !matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = None;
    }
    style
}
pub fn danger_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = secondary_button(theme, status);
    style.text_color = tone(theme, Tone::Danger);
    style
}
pub fn selected_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = secondary_button(theme, status);
    let ink = tone(theme, Tone::Accent);
    style.background = Some(mix(surface(theme), ink, 0.10).into());
    style.text_color = ink;
    style.border.color = ink;
    style
}
pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    style.background = surface(theme).into();
    style.value = theme.palette().text;
    style.placeholder = if dark(theme) {
        color!(0x94A3B8)
    } else {
        color!(0x64748B)
    };
    style.icon = secondary(theme);
    style.border.radius = 6.0.into();
    style.border.color = if matches!(status, text_input::Status::Focused { .. }) {
        tone(theme, Tone::Accent)
    } else {
        color!(0x64748B)
    };
    style
}

/// Small line icons, built once as native RGBA handles; no icon font or SVG engine.
pub fn nav_icon(index: usize, dark_mode: bool) -> image::Handle {
    static ICONS: OnceLock<Vec<image::Handle>> = OnceLock::new();
    let icons = ICONS.get_or_init(|| {
        let shapes: [&[(i32, i32, i32, i32)]; 9] = [
            &[
                (3, 11, 12, 3),
                (12, 3, 21, 11),
                (5, 10, 5, 21),
                (5, 21, 19, 21),
                (19, 21, 19, 10),
                (10, 21, 10, 14),
                (10, 14, 14, 14),
                (14, 14, 14, 21),
            ],
            &[
                (3, 6, 21, 6),
                (3, 12, 21, 12),
                (3, 18, 21, 18),
                (3, 4, 3, 20),
                (21, 4, 21, 20),
            ],
            &[
                (5, 3, 19, 3),
                (19, 3, 19, 21),
                (19, 21, 5, 21),
                (5, 21, 5, 3),
                (8, 8, 16, 8),
                (8, 12, 16, 12),
                (8, 16, 14, 16),
            ],
            &[
                (4, 5, 10, 5),
                (10, 5, 10, 10),
                (10, 10, 4, 10),
                (4, 10, 4, 5),
                (14, 14, 20, 14),
                (20, 14, 20, 20),
                (20, 20, 14, 20),
                (14, 20, 14, 14),
                (8, 10, 17, 14),
            ],
            &[
                (4, 6, 6, 6),
                (10, 6, 21, 6),
                (4, 12, 6, 12),
                (10, 12, 21, 12),
                (4, 18, 6, 18),
                (10, 18, 21, 18),
            ],
            &[
                (5, 3, 19, 3),
                (19, 3, 19, 21),
                (19, 21, 5, 21),
                (5, 21, 5, 3),
                (8, 7, 16, 7),
                (8, 11, 16, 11),
                (8, 15, 16, 15),
                (8, 18, 13, 18),
            ],
            &[
                (2, 12, 6, 12),
                (6, 12, 9, 5),
                (9, 5, 13, 20),
                (13, 20, 16, 10),
                (16, 10, 19, 12),
                (19, 12, 22, 12),
            ],
            &[
                (5, 3, 19, 3),
                (19, 3, 21, 12),
                (21, 12, 12, 22),
                (12, 22, 3, 12),
                (3, 12, 5, 3),
                (9, 8, 15, 8),
                (15, 8, 15, 13),
                (15, 13, 9, 13),
                (9, 13, 9, 8),
            ],
            &[
                (3, 6, 21, 6),
                (3, 12, 21, 12),
                (3, 18, 21, 18),
                (8, 3, 8, 9),
                (16, 9, 16, 15),
                (10, 15, 10, 21),
            ],
        ];
        [false, true]
            .into_iter()
            .flat_map(|dark_mode| {
                shapes.into_iter().map(move |lines| {
                    let color = if dark_mode {
                        [203, 213, 225, 255]
                    } else {
                        [71, 85, 105, 255]
                    };
                    let mut pixels = vec![0u8; 24 * 24 * 4];
                    for &(mut x, mut y, end_x, end_y) in lines {
                        let dx = (end_x - x).abs();
                        let dy = -(end_y - y).abs();
                        let sx = if x < end_x { 1 } else { -1 };
                        let sy = if y < end_y { 1 } else { -1 };
                        let mut error = dx + dy;
                        loop {
                            for offset in [0, 1] {
                                let px = x + offset;
                                if (0..24).contains(&px) && (0..24).contains(&y) {
                                    let i = (y * 24 + px) as usize * 4;
                                    pixels[i..i + 4].copy_from_slice(&color);
                                }
                            }
                            if x == end_x && y == end_y {
                                break;
                            }
                            let twice = 2 * error;
                            if twice >= dy {
                                error += dy;
                                x += sx;
                            }
                            if twice <= dx {
                                error += dx;
                                y += sy;
                            }
                        }
                    }
                    image::Handle::from_rgba(24, 24, pixels)
                })
            })
            .collect()
    });
    icons[index + if dark_mode { 9 } else { 0 }].clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn luminance(color: Color) -> f32 {
        let linear = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }
    #[test]
    fn both_themes_keep_body_and_semantic_text_readable() {
        for dark in [false, true] {
            let theme = theme(dark);
            for ink in [
                theme.palette().text,
                secondary(&theme),
                tone(&theme, Tone::Accent),
                tone(&theme, Tone::Success),
                tone(&theme, Tone::Warning),
                tone(&theme, Tone::Danger),
            ] {
                for background in [surface(&theme), theme.palette().background] {
                    let a = luminance(ink);
                    let b = luminance(background);
                    assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
                }
            }
        }
    }
}
