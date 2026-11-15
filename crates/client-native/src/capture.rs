//! Bildschirmaufnahme für Streams.
//!
//! - **Windows**: GDI (`BitBlt` + `GetDIBits`) auf den Hauptbildschirm. Keine
//!   zusätzliche Bibliothek; bei 1080p wenige Millisekunden pro Bild.
//! - **Linux/X11**: `GetImage` auf das Root-Fenster über `x11rb` (reines Rust).
//! - **Wayland** liefert über XWayland nur ein schwarzes Bild; dort meldet die
//!   Aufnahme einen Fehler statt still Schwarz zu senden.
//!
//! Ergebnis ist immer BGRA, oben links beginnend, mit geraden Kantenlängen
//! (YUV 4:2:0 braucht das) und höchstens [`MAX_WIDTH`] breit.

/// Breitere Bildschirme werden verkleinert (1080p-Budget, siehe DECISIONS).
pub const MAX_WIDTH: usize = 1920;

/// Ein aufgenommenes Bild in BGRA.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub bgra: Vec<u8>,
}

pub trait Capturer: Send {
    fn grab(&mut self) -> Result<Frame, String>;
}

pub fn open() -> Result<Box<dyn Capturer>, String> {
    platform::open()
}

/// Auf gerade Maße zuschneiden und bei Bedarf mit Nächster-Nachbar-Verfahren
/// auf [`MAX_WIDTH`] verkleinern. Text bleibt so scharf genug, und es kostet
/// fast nichts.
pub fn fit(width: usize, height: usize, stride: usize, src: &[u8]) -> Frame {
    let (out_w, out_h) = if width > MAX_WIDTH {
        (MAX_WIDTH, height * MAX_WIDTH / width)
    } else {
        (width, height)
    };
    let out_w = out_w & !1;
    let out_h = out_h & !1;
    let mut bgra = vec![0u8; out_w * out_h * 4];
    if out_w == 0 || out_h == 0 {
        return Frame {
            width: out_w,
            height: out_h,
            bgra,
        };
    }
    for y in 0..out_h {
        let sy = y * height / out_h.max(1);
        let row = &src[sy * stride..];
        let dst = &mut bgra[y * out_w * 4..(y + 1) * out_w * 4];
        if out_w == width {
            dst.copy_from_slice(&row[..out_w * 4]);
        } else {
            for x in 0..out_w {
                let sx = x * width / out_w;
                dst[x * 4..x * 4 + 4].copy_from_slice(&row[sx * 4..sx * 4 + 4]);
            }
        }
    }
    Frame {
        width: out_w,
        height: out_h,
        bgra,
    }
}

#[cfg(windows)]
mod platform {
    use super::{Capturer, Frame, fit};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC,
        ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    struct Gdi {
        screen: HDC,
        memory: HDC,
        bitmap: HBITMAP,
        width: i32,
        height: i32,
        buffer: Vec<u8>,
    }

    // Die Handles gehören allein diesem Objekt und werden nur im
    // Aufnahme-Thread benutzt.
    unsafe impl Send for Gdi {}

    pub fn open() -> Result<Box<dyn Capturer>, String> {
        // SAFETY: reine Abfragen bzw. Anlegen von GDI-Objekten; jedes Handle
        // wird geprüft und in `Drop` freigegeben.
        unsafe {
            let width = GetSystemMetrics(SM_CXSCREEN);
            let height = GetSystemMetrics(SM_CYSCREEN);
            if width <= 0 || height <= 0 {
                return Err("Bildschirmgröße unbekannt".into());
            }
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return Err("Kein Zugriff auf den Bildschirm".into());
            }
            let memory = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            if memory.is_null() || bitmap.is_null() {
                ReleaseDC(std::ptr::null_mut(), screen);
                return Err("GDI-Puffer konnte nicht angelegt werden".into());
            }
            SelectObject(memory, bitmap);
            Ok(Box::new(Gdi {
                screen,
                memory,
                bitmap,
                width,
                height,
                buffer: vec![0; width as usize * height as usize * 4],
            }))
        }
    }

    impl Capturer for Gdi {
        fn grab(&mut self) -> Result<Frame, String> {
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: self.width,
                    // Negativ = Zeilen von oben nach unten.
                    biHeight: -self.height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [unsafe { std::mem::zeroed() }],
            };
            // SAFETY: Handles sind gültig (siehe `open`), der Puffer fasst
            // genau width × height × 4 Bytes.
            let lines = unsafe {
                if BitBlt(
                    self.memory,
                    0,
                    0,
                    self.width,
                    self.height,
                    self.screen,
                    0,
                    0,
                    SRCCOPY | CAPTUREBLT,
                ) == 0
                {
                    return Err("Bildschirm konnte nicht kopiert werden".into());
                }
                GetDIBits(
                    self.memory,
                    self.bitmap,
                    0,
                    self.height as u32,
                    self.buffer.as_mut_ptr().cast(),
                    &mut info,
                    DIB_RGB_COLORS,
                )
            };
            if lines <= 0 {
                return Err("Bildschirm konnte nicht gelesen werden".into());
            }
            let (w, h) = (self.width as usize, self.height as usize);
            Ok(fit(w, h, w * 4, &self.buffer))
        }
    }

    impl Drop for Gdi {
        fn drop(&mut self) {
            // SAFETY: jedes Handle stammt aus `open` und wird genau einmal freigegeben.
            unsafe {
                DeleteObject(self.bitmap);
                DeleteDC(self.memory);
                ReleaseDC(std::ptr::null_mut(), self.screen);
            }
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::{Capturer, Frame, fit};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Window};
    use x11rb::rust_connection::RustConnection;

    struct X11 {
        conn: RustConnection,
        root: Window,
        width: u16,
        height: u16,
    }

    pub fn open() -> Result<Box<dyn Capturer>, String> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some()
            && std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
        {
            return Err(
                "Bildschirm teilen unter Wayland geht noch nicht – bitte eine X11-Sitzung nutzen."
                    .into(),
            );
        }
        let (conn, screen) =
            x11rb::connect(None).map_err(|err| format!("X11 nicht erreichbar: {err}"))?;
        let info = &conn.setup().roots[screen];
        let (root, width, height) = (info.root, info.width_in_pixels, info.height_in_pixels);
        Ok(Box::new(X11 {
            conn,
            root,
            width,
            height,
        }))
    }

    impl Capturer for X11 {
        fn grab(&mut self) -> Result<Frame, String> {
            let reply = self
                .conn
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    self.root,
                    0,
                    0,
                    self.width,
                    self.height,
                    !0,
                )
                .map_err(|err| err.to_string())?
                .reply()
                .map_err(|err| format!("Aufnahme fehlgeschlagen: {err}"))?;
            let (w, h) = (usize::from(self.width), usize::from(self.height));
            // 24/32 Bit Tiefe: 4 Bytes pro Pixel, Reihenfolge B, G, R, X.
            if reply.data.len() < w * h * 4 {
                return Err("Unerwartetes Bildformat (nur 24/32 Bit)".into());
            }
            Ok(fit(w, h, w * 4, &reply.data))
        }
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
mod platform {
    use super::Capturer;

    pub fn open() -> Result<Box<dyn Capturer>, String> {
        Err("Bildschirm teilen gibt es auf diesem System noch nicht.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_small_even_frames() {
        let src = vec![7u8; 4 * 4 * 4];
        let frame = fit(4, 4, 16, &src);
        assert_eq!((frame.width, frame.height), (4, 4));
        assert_eq!(frame.bgra, src);
    }

    #[test]
    fn fit_makes_sizes_even_and_caps_width() {
        let frame = fit(5, 3, 20, &[1u8; 5 * 3 * 4]);
        assert_eq!((frame.width, frame.height), (4, 2));
        let wide = fit(3840, 2160, 3840 * 4, &vec![0u8; 3840 * 2160 * 4]);
        assert_eq!((wide.width, wide.height), (1920, 1080));
        assert_eq!(wide.bgra.len(), 1920 * 1080 * 4);
    }
}
