//! Farben und Stile „Funkraum“ — exakt die Dark-Tokens aus
//! `crates/ui/assets/app.css` (Test `tokens_match_web_css` hält beide gleich).
//! Nova-Gelb heißt nur: live, aktiv, jetzt.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, color, font};

/// Atkinson Hyperlegible Next, dieselbe Schrift wie im Web (TTF, weil iced
/// kein woff2 liest). Eingebunden in `main.rs`.
pub const FONT: Font = Font::with_name("Atkinson Hyperlegible Next");
/// Für Namen und Überschriften; nur 400 und 700 (600 wirkt zu weich).
pub const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..FONT
};

/// `--nv-bg-0`: Server-Leiste
pub const RAIL: Color = color!(0x10151e);
/// `--nv-bg-1`: Kanäle, Mitglieder
pub const SIDEBAR: Color = color!(0x151b26);
/// `--nv-bg-2`: Chat
pub const MAIN: Color = color!(0x1a2130);
/// `--nv-bg-3`: Eingaben, Kacheln, Hover
pub const RAISED: Color = color!(0x212938);
/// `--nv-bg-4`: aktiv, Popover
pub const RAISED_HI: Color = color!(0x2a3446);

/// `--nv-text-1`
pub const TEXT: Color = color!(0xe8ecf3);
/// `--nv-text-2`
pub const MUTED: Color = color!(0xa9b2c3);
/// `--nv-text-3`
pub const FAINT: Color = color!(0x8692a7);
/// `--nv-text-on-accent`: Text auf Gelb, nie Weiß
pub const ON_ACCENT: Color = color!(0x1b1405);

/// `--nv-accent`
pub const ACCENT: Color = color!(0xffc34d);
/// `--nv-accent-hover`
pub const ACCENT_HOVER: Color = color!(0xffd27a);
/// `--nv-speaking`: Sprecher-Ring
pub const SPEAKING: Color = color!(0xffc34d);
/// `--nv-online`
pub const ONLINE: Color = color!(0x4fd18b);
/// `--nv-danger`
pub const DANGER: Color = color!(0xff6b6b);
/// `--nv-offline`
pub const OFFLINE: Color = color!(0x5d6779);

/// Hover-Schleier: macht aus `--nv-bg-1` ungefähr `--nv-bg-3`.
const HOVER: Color = Color {
    r: 0.784,
    g: 0.843,
    b: 0.941,
    a: 0.07,
};
/// `--nv-accent-soft`: ausgewählte Zeile
const ACTIVE: Color = Color {
    r: 1.0,
    g: 0.765,
    b: 0.302,
    a: 0.13,
};
/// `--nv-line`
const BORDER: Color = Color {
    r: 0.784,
    g: 0.843,
    b: 0.941,
    a: 0.08,
};
/// `--nv-line-strong`
const BORDER_STRONG: Color = Color {
    r: 0.784,
    g: 0.843,
    b: 0.941,
    a: 0.14,
};

pub fn theme() -> Theme {
    Theme::custom(
        "Nova",
        iced::theme::Palette {
            background: MAIN,
            text: TEXT,
            primary: ACCENT,
            success: ONLINE,
            warning: DANGER,
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

/// Voice-Kachel; wer spricht, bekommt den 2-px-Ring in Nova-Gelb.
pub fn voice_tile(speaking: bool) -> container::Style {
    container::Style {
        border: Border {
            color: if speaking { SPEAKING } else { BORDER },
            width: if speaking { 2.0 } else { 1.0 },
            radius: 12.0.into(),
        },
        ..fill(RAISED)
    }
}

/// Namensbalken in der Kachel: füllt sich gelb, solange die Person spricht —
/// die einzige größere gelbe Fläche der App.
pub fn name_bar(speaking: bool) -> container::Style {
    container::Style {
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        background: speaking.then_some(Background::Color(ACCENT)),
        ..container::Style::default()
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
            radius: 12.0.into(),
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

/// Reaktions-Pille; eigene Reaktion mit 1-px-Rand in Akzentfarbe.
pub fn reaction(_: &Theme, status: button::Status, me: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(if hovered { RAISED_HI } else { RAISED })),
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

/// Kleines Etikett: „Live“ in Gelb, „Bot“ neutral in `RAISED_HI`.
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
            radius: 12.0.into(),
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

/// `--nv-tone-0…5`: gedämpftes Blau, Grau, Grün, Rost, Petrol, Pflaume — nie gelb.
const AVATAR_TINTS: [Color; 6] = [
    color!(0x2b3a55),
    color!(0x343b48),
    color!(0x27403a),
    color!(0x4a3129),
    color!(0x1f4048),
    color!(0x3e2e48),
];

/// Fester Ton je Name, gleiche Rechnung wie `tone()` in `crates/ui`, damit
/// dieselbe Person in beiden Clients gleich aussieht.
pub fn avatar_tint(name: &str) -> Color {
    let tone = name.bytes().fold(7u32, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(u32::from(byte))
    }) % 6;
    AVATAR_TINTS[tone as usize]
}

/// Avatar mit 2-px-Ring: Sprecher-Gelb oder Flächenfarbe (trennt überlappende).
pub fn avatar_ring(tint: Color, ring: Color) -> container::Style {
    container::Style {
        border: Border {
            color: ring,
            width: 2.0,
            radius: 999.0.into(),
        },
        ..fill(tint)
    }
}

/// Status über Form statt nur Farbe: online gefüllt, offline hohler Ring.
pub fn presence(online: bool) -> container::Style {
    container::Style {
        border: Border {
            color: OFFLINE,
            width: if online { 0.0 } else { 2.0 },
            radius: 999.0.into(),
        },
        background: online.then_some(Background::Color(ONLINE)),
        ..container::Style::default()
    }
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

/// Fläche hinter Icons in Leerzuständen und im Kanal-Willkommen.
pub fn badge(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: 12.0.into(),
            ..Border::default()
        },
        ..fill(RAISED)
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
            radius: 8.0.into(),
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
            radius: 8.0.into(),
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
            base_button(Some(ACCENT_HOVER), ON_ACCENT, 8.0)
        }
        button::Status::Disabled => base_button(Some(RAISED), FAINT, 8.0),
        button::Status::Active => base_button(Some(ACCENT), ON_ACCENT, 8.0),
    }
}

pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => base_button(Some(HOVER), TEXT, 8.0),
        _ => base_button(None, MUTED, 8.0),
    }
}

pub fn link(_: &Theme, status: button::Status) -> button::Style {
    let color = if matches!(status, button::Status::Hovered) {
        ACCENT_HOVER
    } else {
        ACCENT
    };
    base_button(None, color, 8.0)
}

/// Quadratischer Icon-Knopf; `on` markiert einen eingeschalteten Schalter.
pub fn icon_button(_: &Theme, status: button::Status, on: bool) -> button::Style {
    let background = match status {
        _ if on => Some(ACTIVE),
        button::Status::Hovered | button::Status::Pressed => Some(HOVER),
        _ => None,
    };
    base_button(background, MUTED, 8.0)
}

/// Icon-Knopf für Gefährliches (Abmelden): färbt sich beim Hover rot ein.
pub fn danger_icon_button(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Some(Color { a: 0.14, ..DANGER }),
        _ => None,
    };
    base_button(background, MUTED, 8.0)
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
    base_button(background, TEXT, 8.0)
}

/// Server in der linken Leiste: immer abgerundetes Quadrat (44 px, Radius 12),
/// kein Kreis-zu-Quadrat. Ausgewählt: `RAISED_HI` plus 2-px-Ring in Gelb.
pub fn rail_button(_: &Theme, status: button::Status, active: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let background = if active || hovered { RAISED_HI } else { RAISED };
    button::Style {
        border: Border {
            color: if active { ACCENT } else { Color::TRANSPARENT },
            width: if active { 2.0 } else { 0.0 },
            radius: 12.0.into(),
        },
        ..base_button(
            Some(background),
            if active || hovered { TEXT } else { MUTED },
            12.0,
        )
    }
}

/// 2-px-Streifen links am ausgewählten Kanal.
pub fn accent_stripe(_: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: 2.0.into(),
            ..Border::default()
        },
        ..fill(ACCENT)
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
            ("tooltip", tooltip),
            ("composer", composer),
            ("accent_stripe", accent_stripe),
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

    /// Liest einen Farbwert (`#rrggbb` oder `rgba(r, g, b, a)`) als `Color`.
    fn parse_css_color(value: &str) -> Color {
        if let Some(hex) = value.strip_prefix('#') {
            let rgb = u32::from_str_radix(hex, 16).expect("Hex-Farbe");
            return Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        }
        let inner = value
            .strip_prefix("rgba(")
            .and_then(|v| v.strip_suffix(')'))
            .expect("rgba(…)");
        let parts: Vec<f32> = inner
            .split(',')
            .map(|p| p.trim().parse().expect("Zahl"))
            .collect();
        Color::from_rgba8(parts[0] as u8, parts[1] as u8, parts[2] as u8, parts[3])
    }

    /// Die Konstanten müssen den Dark-Tokens in `app.css` entsprechen, sonst
    /// laufen Web und nativer Client wieder auseinander.
    #[test]
    fn tokens_match_web_css() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/assets/app.css");
        let css = std::fs::read_to_string(path).expect("app.css lesbar");
        // Nur der erste `:root`-Block (Dark); Light folgt später in der Datei.
        let root = css
            .split(":root {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect(":root-Block");
        let token = |name: &str| -> Color {
            let line = root
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with(&format!("{name}:")))
                .unwrap_or_else(|| panic!("{name} fehlt in app.css"));
            let value = line[name.len() + 1..].trim().trim_end_matches(';').trim();
            parse_css_color(value)
        };
        let mut pairs = vec![
            ("--nv-bg-0", RAIL),
            ("--nv-bg-1", SIDEBAR),
            ("--nv-bg-2", MAIN),
            ("--nv-bg-3", RAISED),
            ("--nv-bg-4", RAISED_HI),
            ("--nv-line", BORDER),
            ("--nv-line-strong", BORDER_STRONG),
            ("--nv-text-1", TEXT),
            ("--nv-text-2", MUTED),
            ("--nv-text-3", FAINT),
            ("--nv-text-on-accent", ON_ACCENT),
            ("--nv-accent", ACCENT),
            ("--nv-accent-hover", ACCENT_HOVER),
            ("--nv-accent-soft", ACTIVE),
            ("--nv-speaking", SPEAKING),
            ("--nv-online", ONLINE),
            ("--nv-danger", DANGER),
            ("--nv-offline", OFFLINE),
        ];
        let tones = [
            "--nv-tone-0",
            "--nv-tone-1",
            "--nv-tone-2",
            "--nv-tone-3",
            "--nv-tone-4",
            "--nv-tone-5",
        ];
        pairs.extend(tones.into_iter().zip(AVATAR_TINTS));
        for (name, native) in pairs {
            let web = token(name);
            let close = |a: f32, b: f32, eps: f32| (a - b).abs() <= eps;
            assert!(
                close(web.r, native.r, 1.0 / 255.0)
                    && close(web.g, native.g, 1.0 / 255.0)
                    && close(web.b, native.b, 1.0 / 255.0)
                    && close(web.a, native.a, 0.005),
                "{name}: app.css {web:?} ≠ nativ {native:?}"
            );
        }
    }
}
