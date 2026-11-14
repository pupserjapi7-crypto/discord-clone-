//! Linien-Icons, direkt als Vektorpfade gezeichnet.
//!
//! Kein `svg`-Feature (zöge resvg samt Schriftdatenbank nach) und keine
//! Icon-Schrift: Die Pfade liegen in einem 24×24-Raster, werden auf die
//! Zielgröße skaliert und über das `canvas`-Feature mit tiny-skia gezeichnet.
//! Formensprache angelehnt an Lucide (ISC-Lizenz): 2 px Strich, runde Enden.

use std::f32::consts::{FRAC_PI_4, PI};

use iced::widget::canvas::{self, Frame, Geometry, LineCap, LineJoin, Path, Stroke, path};
use iced::{Color, Element, Point, Radians, Rectangle, Renderer, Theme, mouse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Nova-Bildmarke (gefüllter Stern), keine Schaltfläche.
    Logo,
    Hash,
    Volume,
    LogOut,
    Users,
    Send,
    Download,
    Refresh,
    Message,
    Close,
}

struct Glyph {
    icon: Icon,
    color: Color,
    hover: Option<Color>,
}

pub fn icon<'a, M: 'a>(icon: Icon, size: f32, color: Color) -> Element<'a, M> {
    canvas::Canvas::new(Glyph {
        icon,
        color,
        hover: None,
    })
    .width(size)
    .height(size)
    .into()
}

/// Wie [`icon`], färbt sich aber unter dem Mauszeiger um.
pub fn icon_hover<'a, M: 'a>(icon: Icon, size: f32, color: Color, hover: Color) -> Element<'a, M> {
    canvas::Canvas::new(Glyph {
        icon,
        color,
        hover: Some(hover),
    })
    .width(size)
    .height(size)
    .into()
}

impl<M> canvas::Program<M> for Glyph {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let scale = bounds.width.min(bounds.height) / 24.0;
        let color = match self.hover {
            Some(hover) if cursor.is_over(bounds) => hover,
            _ => self.color,
        };
        let stroke = Stroke::default()
            .with_color(color)
            .with_width((2.0 * scale).max(1.4))
            .with_line_cap(LineCap::Round)
            .with_line_join(LineJoin::Round);
        let p = |x: f32, y: f32| Point::new(x * scale, y * scale);
        let arc = |b: &mut path::Builder, cx: f32, cy: f32, r: f32, from: f32, to: f32| {
            b.arc(path::Arc {
                center: p(cx, cy),
                radius: r * scale,
                start_angle: Radians(from),
                end_angle: Radians(to),
            });
        };

        match self.icon {
            Icon::Logo => {
                let star = Path::new(|b| {
                    b.move_to(p(12.0, 2.0));
                    b.quadratic_curve_to(p(13.4, 10.6), p(22.0, 12.0));
                    b.quadratic_curve_to(p(13.4, 13.4), p(12.0, 22.0));
                    b.quadratic_curve_to(p(10.6, 13.4), p(2.0, 12.0));
                    b.quadratic_curve_to(p(10.6, 10.6), p(12.0, 2.0));
                    b.close();
                });
                frame.fill(&star, color);
                frame.fill(&Path::circle(p(19.5, 4.5), 1.6 * scale), color);
            }
            Icon::Hash => {
                let path = Path::new(|b| {
                    for (a, z) in [
                        ((4.0, 9.0), (20.0, 9.0)),
                        ((4.0, 15.0), (20.0, 15.0)),
                        ((10.0, 3.0), (8.0, 21.0)),
                        ((16.0, 3.0), (14.0, 21.0)),
                    ] {
                        b.move_to(p(a.0, a.1));
                        b.line_to(p(z.0, z.1));
                    }
                });
                frame.stroke(&path, stroke);
            }
            Icon::Volume => {
                let path = Path::new(|b| {
                    b.move_to(p(11.0, 5.0));
                    b.line_to(p(6.0, 9.0));
                    b.line_to(p(2.0, 9.0));
                    b.line_to(p(2.0, 15.0));
                    b.line_to(p(6.0, 15.0));
                    b.line_to(p(11.0, 19.0));
                    b.close();
                });
                frame.stroke(&path, stroke);
                let waves = Path::new(|b| {
                    arc(b, 12.0, 12.0, 4.0, -FRAC_PI_4, FRAC_PI_4);
                });
                frame.stroke(&waves, stroke);
                let outer = Path::new(|b| {
                    arc(b, 12.0, 12.0, 8.5, -FRAC_PI_4, FRAC_PI_4);
                });
                frame.stroke(&outer, stroke);
            }
            Icon::LogOut => {
                let path = Path::new(|b| {
                    b.move_to(p(9.0, 21.0));
                    b.arc_to(p(3.0, 21.0), p(3.0, 3.0), 2.0 * scale);
                    b.arc_to(p(3.0, 3.0), p(9.0, 3.0), 2.0 * scale);
                    b.line_to(p(9.0, 3.0));
                    b.move_to(p(16.0, 17.0));
                    b.line_to(p(21.0, 12.0));
                    b.line_to(p(16.0, 7.0));
                    b.move_to(p(21.0, 12.0));
                    b.line_to(p(9.0, 12.0));
                });
                frame.stroke(&path, stroke);
            }
            Icon::Users => {
                frame.stroke(&Path::circle(p(9.0, 7.0), 4.0 * scale), stroke);
                let path = Path::new(|b| {
                    b.move_to(p(16.0, 21.0));
                    b.arc_to(p(16.0, 15.0), p(2.0, 15.0), 4.0 * scale);
                    b.arc_to(p(2.0, 15.0), p(2.0, 21.0), 4.0 * scale);
                    b.line_to(p(2.0, 21.0));
                    b.move_to(p(22.0, 21.0));
                    b.line_to(p(22.0, 19.0));
                    b.quadratic_curve_to(p(22.0, 16.0), p(19.0, 15.1));
                });
                frame.stroke(&path, stroke);
                let back = Path::new(|b| {
                    arc(b, 15.0, 7.0, 4.0, -1.318, 1.318);
                });
                frame.stroke(&back, stroke);
            }
            Icon::Send => {
                let path = Path::new(|b| {
                    b.move_to(p(22.0, 2.0));
                    b.line_to(p(15.0, 22.0));
                    b.line_to(p(11.0, 13.0));
                    b.line_to(p(2.0, 9.0));
                    b.close();
                    b.move_to(p(22.0, 2.0));
                    b.line_to(p(11.0, 13.0));
                });
                frame.stroke(&path, stroke);
            }
            Icon::Download => {
                let path = Path::new(|b| {
                    b.move_to(p(12.0, 3.0));
                    b.line_to(p(12.0, 15.0));
                    b.move_to(p(7.0, 10.0));
                    b.line_to(p(12.0, 15.0));
                    b.line_to(p(17.0, 10.0));
                    b.move_to(p(21.0, 15.0));
                    b.arc_to(p(21.0, 21.0), p(3.0, 21.0), 2.0 * scale);
                    b.arc_to(p(3.0, 21.0), p(3.0, 15.0), 2.0 * scale);
                    b.line_to(p(3.0, 15.0));
                });
                frame.stroke(&path, stroke);
            }
            Icon::Refresh => {
                let path = Path::new(|b| {
                    // Dreiviertelkreis im Uhrzeigersinn, endet oben rechts.
                    arc(b, 12.0, 12.0, 9.0, 0.0, 1.75 * PI);
                    b.line_to(p(21.0, 8.0));
                    b.move_to(p(21.0, 3.0));
                    b.line_to(p(21.0, 8.0));
                    b.line_to(p(16.0, 8.0));
                });
                frame.stroke(&path, stroke);
            }
            Icon::Message => {
                let path = Path::new(|b| {
                    b.move_to(p(21.0, 12.0));
                    b.arc_to(p(21.0, 17.0), p(3.0, 17.0), 2.0 * scale);
                    b.line_to(p(7.0, 17.0));
                    b.line_to(p(3.0, 21.0));
                    b.arc_to(p(3.0, 3.0), p(21.0, 3.0), 2.0 * scale);
                    b.arc_to(p(21.0, 3.0), p(21.0, 17.0), 2.0 * scale);
                    b.close();
                });
                frame.stroke(&path, stroke);
            }
            Icon::Close => {
                let path = Path::new(|b| {
                    b.move_to(p(18.0, 6.0));
                    b.line_to(p(6.0, 18.0));
                    b.move_to(p(6.0, 6.0));
                    b.line_to(p(18.0, 18.0));
                });
                frame.stroke(&path, stroke);
            }
        }
        vec![frame.into_geometry()]
    }
}
