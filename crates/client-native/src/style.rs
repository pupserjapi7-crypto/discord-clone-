//! Farben und Stile — dieselben Tokens wie `crates/ui/assets/app.css`.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, color, font};

/// Für Namen und Überschriften: trägt Hierarchie ohne größere Schrift.
pub const SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

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

/// Gedämpfte Avatar-Töne: unterscheidbar, aber keiner lauter als der Akzent.
const AVATAR_TINTS: [Color; 8] = [
    color!(0x4c5499),
    color!(0x2f6f5a),
    color!(0x80562f),
    color!(0x7a3f62),
    color!(0x2f6587),
    color!(0x67478c),
    color!(0x3d6d72),
    color!(0x84484c),
];

/// Fester Ton je Name, damit dieselbe Person überall gleich aussieht.
pub fn avatar_tint(name: &str) -> Color {
    let hash = name.bytes().fold(2_166_136_261_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    });
    AVATAR_TINTS[hash as usize % AVATAR_TINTS.len()]
}

pub fn avatar(tint: Color) -> container::Style {
    container::Style {
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..fill(tint)
    }
}

/// Statuspunkt am Avatar; der Ring in Flächenfarbe schneidet ihn frei.
pub fn status_dot(color: Color, ring: Color) -> container::Style {
    container::Style {
        border: Border {
            color: ring,
            width: 2.5,
            radius: 999.0.into(),
        },
        ..fill(color)
    }
}

pub fn divider(_: &Theme) -> container::Style {
    fill(BORDER)
}

/// Runde Fläche hinter Icons in Leerzuständen.
pub fn badge(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..fill(ACTIVE)
    }
}

/// Markenfläche auf dem Login-Bildschirm.
pub fn brand_soft(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: 16.0.into(),
            ..Border::default()
        },
        ..fill(ACTIVE)
    }
}

pub fn tooltip(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER_STRONG,
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..fill(RAIL)
    }
}

/// Eingabezeile unten: Feld und Senden-Knopf teilen sich eine Fläche.
pub fn composer(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER_STRONG,
            width: 1.0,
            radius: 10.0.into(),
        },
        ..fill(RAISED)
    }
}

/// Textfeld ohne eigenen Rahmen, sitzt in [`composer`].
pub fn bare_input(_: &Theme, _status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: MUTED,
        placeholder: FAINT,
        value: TEXT,
        selection: ACTIVE,
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

/// Quadratischer Icon-Knopf; `on` markiert einen eingeschalteten Schalter.
pub fn icon_button(_: &Theme, status: button::Status, on: bool) -> button::Style {
    let background = match status {
        _ if on => Some(ACTIVE),
        button::Status::Hovered | button::Status::Pressed => Some(HOVER),
        _ => None,
    };
    base_button(background, MUTED, 6.0)
}

/// Icon-Knopf für Gefährliches (Abmelden): färbt sich beim Hover rot ein.
pub fn danger_icon_button(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Some(Color { a: 0.14, ..DANGER }),
        _ => None,
    };
    base_button(background, MUTED, 6.0)
}

/// Senden-Knopf im Composer: Akzentfläche nur, wenn es etwas zu senden gibt.
pub fn send_button(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => {
            base_button(Some(ACCENT_HOVER), ON_ACCENT, 8.0)
        }
        button::Status::Active => base_button(Some(ACCENT), ON_ACCENT, 8.0),
        button::Status::Disabled => base_button(None, FAINT, 8.0),
    }
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

/// Server in der linken Leiste: rund im Ruhezustand, abgerundetes Quadrat bei
/// Hover, aktiv mit Akzentring statt voller Akzentfläche — so bleibt der Akzent
/// für Aktionen reserviert und das Logo darüber verwechselt man nicht.
pub fn rail_button(_: &Theme, status: button::Status, active: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let (background, radius, border) = if active {
        (ACTIVE, 14.0, ACCENT)
    } else if hovered {
        (color!(0x2a303b), 14.0, Color::TRANSPARENT)
    } else {
        (RAISED, 24.0, Color::TRANSPARENT)
    };
    button::Style {
        border: Border {
            color: border,
            width: if active { 2.0 } else { 0.0 },
            radius: radius.into(),
        },
        ..base_button(Some(background), TEXT, radius)
    }
}
