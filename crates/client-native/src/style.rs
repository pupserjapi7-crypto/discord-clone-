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

/// Voice-Kachel; sprechende Personen bekommen einen 2-px-Ring in Grün.
pub fn voice_tile(speaking: bool) -> container::Style {
    container::Style {
        border: Border {
            color: if speaking { ONLINE } else { BORDER },
            width: if speaking { 2.0 } else { 1.0 },
            radius: 14.0.into(),
        },
        ..fill(RAISED)
    }
}

pub fn voice_bar(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..fill(SIDEBAR)
    }
}

pub fn danger_button(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0xe8848d),
        _ => DANGER,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: ON_ACCENT,
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Abgedunkelte Fläche hinter Dialogen.
pub fn backdrop(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color {
            a: 0.62,
            ..Color::BLACK
        })),
        ..container::Style::default()
    }
}

/// Schatten sind im nativen Client verboten.
///
/// iced_tiny_skia 0.14.1 malt Schatten ohne Clip-Maske über das ganze
/// Schattenrechteck (inklusive der Fläche unter dem Element), auch wenn nur ein
/// kleiner Bereich neu gezeichnet wird. Bei jeder Teil-Neuzeichnung (Hover über
/// einen Knopf, blinkender Cursor) legt sich der halbtransparente Schatten
/// erneut über die unveränderten Pixel; nach ein paar Durchgängen wird die
/// Fläche schwarz. Tiefe entsteht deshalb nur über Flächenfarbe und
/// `BORDER_STRONG`. Der Test `styles_have_no_shadows` hält das fest.
pub const NO_SHADOW: Shadow = Shadow {
    color: Color::TRANSPARENT,
    offset: iced::Vector::ZERO,
    blur_radius: 0.0,
};

/// Dialogkarte über dem Backdrop.
pub fn modal(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER_STRONG,
            width: 1.0,
            radius: 14.0.into(),
        },
        // Kein Schatten: siehe `NO_SHADOW`.
        shadow: NO_SHADOW,
        ..fill(SIDEBAR)
    }
}

/// Schwebende Aktionsleiste an einer Nachricht (beim Überfahren).
pub fn message_tools(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER_STRONG,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..fill(RAISED)
    }
}

/// Hervorhebung der überfahrenen Nachricht.
pub fn message_hover(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color {
            a: 0.025,
            ..Color::WHITE
        })),
        ..container::Style::default()
    }
}

/// Reaktions-Pille; eigene Reaktion in Akzentfarbe umrandet.
pub fn reaction(_: &Theme, status: button::Status, me: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(if me {
            ACTIVE
        } else if hovered {
            BORDER_STRONG
        } else {
            RAISED
        })),
        text_color: TEXT,
        border: Border {
            color: if me { ACCENT } else { BORDER },
            width: 1.0,
            radius: 999.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Kleines Etikett (z. B. „BOT“) in voller Farbe.
pub fn tag(color: Color) -> container::Style {
    container::Style {
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        background: Some(Background::Color(color)),
        text_color: Some(ON_ACCENT),
        ..container::Style::default()
    }
}

/// Fläche hinter einem Stream (Letterbox schwarz).
pub fn stage(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::BLACK)),
        border: Border {
            radius: 0.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Leiste über dem Stream (Name, LIVE, Knöpfe).
pub fn stage_bar(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..fill(RAISED)
    }
}

/// Kleiner Ungelesen-Punkt bzw. Pille.
pub fn unread_dot(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: 999.0.into(),
            ..Border::default()
        },
        ..fill(TEXT)
    }
}

/// Wichtiger Hinweis im Dialog (z. B. Bestätigung vor Bann).
pub fn danger_note(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: DANGER,
            width: 1.0,
            radius: 8.0.into(),
        },
        background: Some(Background::Color(Color { a: 0.10, ..DANGER })),
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
        shadow: NO_SHADOW,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Schatten werden in iced_tiny_skia bei Teil-Neuzeichnungen immer wieder
    /// übereinandergemalt und färben Flächen schwarz (siehe `NO_SHADOW`).
    #[test]
    fn styles_have_no_shadows() {
        let theme = theme();
        type ContainerStyle = fn(&Theme) -> container::Style;
        let containers: [(&str, ContainerStyle); 19] = [
            ("voice_bar", voice_bar),
            ("backdrop", backdrop),
            ("modal", modal),
            ("message_tools", message_tools),
            ("message_hover", message_hover),
            ("stage", stage),
            ("stage_bar", stage_bar),
            ("unread_dot", unread_dot),
            ("danger_note", danger_note),
            ("app_bg", app_bg),
            ("rail", rail),
            ("sidebar", sidebar),
            ("card", card),
            ("toast", toast),
            ("divider", divider),
            ("badge", badge),
            ("brand_soft", brand_soft),
            ("tooltip", tooltip),
            ("composer", composer),
        ];
        for (name, style) in containers {
            assert_eq!(
                style(&theme).shadow.color.a,
                0.0,
                "{name} hat einen Schatten"
            );
        }
        let statuses = [
            button::Status::Active,
            button::Status::Hovered,
            button::Status::Pressed,
            button::Status::Disabled,
        ];
        for status in statuses {
            let buttons = [
                danger_button(&theme, status),
                reaction(&theme, status, true),
                primary(&theme, status),
                ghost(&theme, status),
                link(&theme, status),
                icon_button(&theme, status, true),
                danger_icon_button(&theme, status),
                send_button(&theme, status),
                channel_button(&theme, status, true),
                rail_button(&theme, status, true),
            ];
            for style in buttons {
                assert_eq!(style.shadow.color.a, 0.0, "Knopf mit Schatten");
            }
        }
    }
}
