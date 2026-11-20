//! Anhänge im nativen Client: Dateien wählen, hochladen, Vorschaubilder
//! dekodieren und Dateien speichern.
//!
//! Bilder kommen vom Server ohne Metadaten, aber mit EXIF-Drehung; die wird
//! hier angewendet. Vorschauen werden auf höchstens [`PREVIEW_MAX`] Pixel
//! verkleinert – ein 12-MP-Foto belegt sonst 48 MB RAM.

use std::io::Cursor;

use iced::widget::image::Handle;

use crate::api::{Api, Attachment};

/// Höchstens so viele Anhänge pro Nachricht (wie der Server).
pub const MAX_ATTACHMENTS: usize = 10;

/// Größer liest der Client gar nicht erst ein (Schutz vor Versehen; das
/// eigentliche Limit setzt der Server).
const MAX_PICK_BYTES: u64 = 100 * 1024 * 1024;

/// Anzeigegröße im Chat.
pub const DISPLAY_W: f32 = 360.0;
pub const DISPLAY_H: f32 = 270.0;
/// Vorschau in doppelter Auflösung für HiDPI.
const PREVIEW_MAX: u32 = 720;

/// Datei im Composer.
#[derive(Debug, Clone)]
pub struct PendingFile {
    pub key: u64,
    pub name: String,
    pub size: i64,
    pub state: PendingState,
}

#[derive(Debug, Clone)]
pub enum PendingState {
    Uploading,
    Done(Attachment),
    Failed(String),
}

/// Vorschaubild eines Anhangs.
#[derive(Debug, Clone)]
pub enum Preview {
    Loading,
    Ready(Handle),
    Failed,
}

/// Dekodiertes, verkleinertes Bild (RGBA).
#[derive(Debug, Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Decoded {
    pub fn handle(self) -> Handle {
        Handle::from_rgba(self.width, self.height, self.rgba)
    }
}

/// Gewählte Datei: Name und Inhalt.
#[derive(Debug, Clone)]
pub struct Picked {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Dateien über den System-Dialog wählen. Zu große Dateien werden mit einer
/// Meldung übersprungen.
pub async fn pick_files() -> (Vec<Picked>, Vec<String>) {
    let Some(handles) = rfd::AsyncFileDialog::new()
        .set_title("Dateien anhängen")
        .pick_files()
        .await
    else {
        return (Vec::new(), Vec::new());
    };
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for handle in handles.into_iter().take(MAX_ATTACHMENTS) {
        let name = handle.file_name();
        let size = std::fs::metadata(handle.path())
            .map(|meta| meta.len())
            .unwrap_or(0);
        if size > MAX_PICK_BYTES {
            errors.push(format!("{name}: Die Datei ist zu groß."));
            continue;
        }
        files.push(Picked {
            name,
            bytes: handle.read().await,
        });
    }
    (files, errors)
}

/// Anhang laden und über den Speichern-Dialog ablegen. `Ok(None)`: abgebrochen.
pub async fn save(api: Api, attachment: Attachment) -> Result<Option<String>, String> {
    let Some(handle) = rfd::AsyncFileDialog::new()
        .set_title("Anhang speichern")
        .set_file_name(&attachment.filename)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let bytes = api.download(attachment.id).await?;
    tokio::fs::write(handle.path(), bytes)
        .await
        .map_err(|err| format!("Speichern fehlgeschlagen: {err}"))?;
    Ok(Some(handle.file_name()))
}

/// Bild laden, drehen, verkleinern – im Blocking-Pool, nicht im UI-Thread.
pub async fn load_preview(api: Api, id: i64) -> Result<Decoded, String> {
    let bytes = api.download(id).await?;
    tokio::task::spawn_blocking(move || decode_preview(&bytes))
        .await
        .map_err(|err| err.to_string())?
}

pub fn decode_preview(bytes: &[u8]) -> Result<Decoded, String> {
    use image::{DynamicImage, ImageDecoder, ImageReader, Limits};

    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(320 * 1024 * 1024);
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|err| err.to_string())?;
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|err| err.to_string())?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture = DynamicImage::from_decoder(decoder).map_err(|err| err.to_string())?;
    picture.apply_orientation(orientation);
    let small = if picture.width() > PREVIEW_MAX || picture.height() > PREVIEW_MAX {
        picture.thumbnail(PREVIEW_MAX, PREVIEW_MAX)
    } else {
        picture
    };
    let rgba = small.to_rgba8();
    Ok(Decoded {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// Anzeigegröße: höchstens [`DISPLAY_W`] × [`DISPLAY_H`], Seitenverhältnis bleibt.
pub fn display_size(attachment: &Attachment) -> (f32, f32) {
    let width = attachment.width.unwrap_or(1).max(1) as f32;
    let height = attachment.height.unwrap_or(1).max(1) as f32;
    let scale = (DISPLAY_W / width).min(DISPLAY_H / height).min(1.0);
    (
        (width * scale).round().max(1.0),
        (height * scale).round().max(1.0),
    )
}

/// Dateigröße für Menschen: „820 B“, „14 KB“, „3,2 MB“.
pub fn size_label(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes < 1024.0 {
        return format!("{bytes} B");
    }
    if bytes < 1024.0 * 1024.0 {
        return format!("{} KB", (bytes / 1024.0).round());
    }
    let mb = bytes / (1024.0 * 1024.0);
    let text = if mb < 10.0 {
        format!("{mb:.1}")
    } else {
        format!("{}", mb.round())
    };
    format!("{} MB", text.replace('.', ","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_german() {
        assert_eq!(size_label(820), "820 B");
        assert_eq!(size_label(3_355_443), "3,2 MB");
    }

    #[test]
    fn display_never_upscales() {
        let attachment = Attachment {
            id: 1,
            filename: "a.png".into(),
            content_type: "image/png".into(),
            size: 1,
            width: Some(4000),
            height: Some(3000),
        };
        assert_eq!(display_size(&attachment), (360.0, 270.0));
        let small = Attachment {
            width: Some(32),
            height: Some(16),
            ..attachment
        };
        assert_eq!(display_size(&small), (32.0, 16.0));
    }

    #[test]
    fn previews_are_decoded_and_shrunk() {
        let mut png = Vec::new();
        let picture = image::RgbaImage::from_pixel(1600, 800, image::Rgba([255, 195, 77, 255]));
        image::DynamicImage::ImageRgba8(picture)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode");
        let decoded = decode_preview(&png).expect("decode");
        assert_eq!((decoded.width, decoded.height), (720, 360));
        assert_eq!(decoded.rgba.len(), 720 * 360 * 4);
        assert!(decode_preview(b"kein bild").is_err());
    }
}
