//! Farben und Stile — dieselben Tokens wie `crates/ui/assets/app.css`.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Shadow, Theme, color};

pub const RAIL: Color = color!(0x0f1115);
pub const SIDEBAR: Color = color!(0x13161c);
pub const MAIN: Color = color!(0x171b22);
pub const RAISED: Color = color!(0x1d222b);

pub const TEXT: Color = color!(0xe8ebf1);
pub const MUTED: Color = color!(0xa0a8b7);
pub const FAINT: Color = color!(0x7d8797);
pub const ON_ACCENT: Color = color!(0x10121a);

pub const ACCENT: Color = color!(0x7b86f8);
pub const ACCENT_HOVER: Color = color!(0x929bff);
pub const ONLINE: Color = color!(0x49b98a);
pub const WARN: Color = color!(0xd0a44c);
pub const DANGER: Color = color!(0xdf737d);

const HOVER: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.045,
};
const ACTIVE: Color = Color {
    r: 0.482,
    g: 0.525,
    b: 0.973,
    a: 0.16,
};
const BORDER: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.07,
};
const BORDER_STRONG: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 0.13,
};

pub fn theme() -> Theme {
    Theme::custom(
        "Nova",
        iced::theme::Palette {
            background: MAIN,
            text: TEXT,
            primary: ACCENT,
            success: ONLINE,
            warning: WARN,
            danger: DANGER,
        },
    )
}

fn fill(color: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(color)),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn app_bg(_: &Theme) -> container::Style {
    fill(MAIN)
}

pub fn rail(_: &Theme) -> container::Style {
    fill(RAIL)
}

pub fn sidebar(_: &Theme) -> container::Style {
    fill(SIDEBAR)
}

pub fn me_bar(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..fill(RAIL)
    }
}

pub fn header(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..fill(MAIN)
    }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER_STRONG,
            width: 1.0,
            radius: 16.0.into(),
        },
        ..fill(SIDEBAR)
    }
}

pub fn toast(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: DANGER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..fill(RAISED)
    }
}

pub fn banner(tone: Color) -> container::Style {
    container::Style {
        border: Border {
            color: tone,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..fill(RAISED)
    }
}

pub fn brand(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: 14.0.into(),
            ..Border::default()
        },
        ..fill(ACCENT)
    }
}

pub fn avatar(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..fill(color!(0x3a4150))
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(RAISED),
        border: Border {
            color: if focused { ACCENT } else { BORDER_STRONG },
            width: 1.0,
            radius: 10.0.into(),
        },
        icon: MUTED,
        placeholder: FAINT,
        value: TEXT,
        selection: ACTIVE,
    }
}

fn base_button(background: Option<Color>, text_color: Color, radius: f32) -> button::Style {
    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => {
            base_button(Some(ACCENT_HOVER), ON_ACCENT, 10.0)
        }
        button::Status::Disabled => base_button(Some(RAISED), FAINT, 10.0),
        button::Status::Active => base_button(Some(ACCENT), ON_ACCENT, 10.0),
    }
}

pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => base_button(Some(HOVER), TEXT, 6.0),
        _ => base_button(None, MUTED, 6.0),
    }
}

pub fn link(_: &Theme, status: button::Status) -> button::Style {
    let color = if matches!(status, button::Status::Hovered) {
        ACCENT_HOVER
    } else {
        ACCENT
    };
    base_button(None, color, 6.0)
}

pub fn channel_button(_: &Theme, status: button::Status, active: bool) -> button::Style {
    let background = if active {
        Some(ACTIVE)
    } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        Some(HOVER)
    } else {
        None
    };
    base_button(background, TEXT, 6.0)
}

pub fn rail_button(_: &Theme, status: button::Status, active: bool) -> button::Style {
    let (background, radius) = if active {
        (ACCENT, 14.0)
    } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        (color!(0x2a303b), 14.0)
    } else {
        (RAISED, 24.0)
    };
    base_button(Some(background), TEXT, radius)
}
