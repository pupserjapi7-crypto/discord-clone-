//! Dateityp erkennen und Bild-Metadaten entfernen – ohne Dekodieren.
//!
//! Der Typ kommt nie aus dem `Content-Type` des Clients, sondern aus den
//! ersten Bytes. Nur JPEG, PNG, GIF und WebP gelten als Bild und werden
//! inline angezeigt; alles andere (auch SVG) wird als Download ausgeliefert.
//!
//! Bilder werden nicht neu kodiert (kostet CPU und Qualität), sondern auf
//! Byte-Ebene gefiltert: EXIF (GPS, Kameradaten), XMP, IPTC, Kommentare und
//! Textblöcke fliegen raus, Farbprofile bleiben. Eine EXIF-Drehung bleibt als
//! minimaler EXIF-Block erhalten, sonst stünden Handyfotos auf der Seite.
//! Daten hinter dem Bildende (typisch für Polyglot-Dateien) werden verworfen.

use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};

/// Erkannter Typ einer hochgeladenen Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Gif,
    Webp,
    Mp4,
    QuickTime,
    Webm,
    Ogg,
    Mp3,
    Wav,
    Flac,
    M4a,
    Pdf,
    Zip,
    Text,
    Other,
}

impl Kind {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Mp4 => "video/mp4",
            Self::QuickTime => "video/quicktime",
            Self::Webm => "video/webm",
            Self::Ogg => "audio/ogg",
            Self::Mp3 => "audio/mpeg",
            Self::Wav => "audio/wav",
            Self::Flac => "audio/flac",
            Self::M4a => "audio/mp4",
            Self::Pdf => "application/pdf",
            Self::Zip => "application/zip",
            Self::Text => "text/plain; charset=utf-8",
            Self::Other => "application/octet-stream",
        }
    }

    pub fn is_image(self) -> bool {
        matches!(self, Self::Jpeg | Self::Png | Self::Gif | Self::Webp)
    }
}

/// Wie viele Bytes [`sniff`] mindestens sehen möchte.
pub const SNIFF_LEN: usize = 512;

/// Größte Seitenlänge und Pixelzahl, die noch inline angezeigt wird. Größere
/// Bilder („Pixelbomben“) gibt es nur als Download.
pub const MAX_INLINE_SIDE: u32 = 16_384;
pub const MAX_INLINE_PIXELS: u64 = 60_000_000;

/// Typ aus den ersten Bytes bestimmen. `head` darf kürzer als
/// [`SNIFF_LEN`] sein (kleine Dateien).
pub fn sniff(head: &[u8]) -> Kind {
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Kind::Jpeg;
    }
    if starts(b"\x89PNG\r\n\x1a\n") {
        return Kind::Png;
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Kind::Gif;
    }
    if head.len() >= 12 && starts(b"RIFF") {
        match &head[8..12] {
            b"WEBP" => return Kind::Webp,
            b"WAVE" => return Kind::Wav,
            _ => {}
        }
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return match &head[8..12] {
            b"qt  " => Kind::QuickTime,
            b"M4A " | b"M4B " => Kind::M4a,
            // HEIC/AVIF zeigt nicht jeder Browser an: Download.
            b"heic" | b"heix" | b"mif1" | b"msf1" | b"avif" | b"avis" => Kind::Other,
            _ => Kind::Mp4,
        };
    }
    if starts(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Kind::Webm;
    }
    if starts(b"OggS") {
        return Kind::Ogg;
    }
    if starts(b"fLaC") {
        return Kind::Flac;
    }
    if starts(b"ID3") || (head.len() >= 2 && head[0] == 0xFF && (head[1] & 0xE6) == 0xE2) {
        return Kind::Mp3;
    }
    if starts(b"%PDF-") {
        return Kind::Pdf;
    }
    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") {
        return Kind::Zip;
    }
    if looks_like_text(head) {
        return Kind::Text;
    }
    Kind::Other
}

/// UTF-8 ohne NUL und ohne Steuerzeichen außer Tab/Zeilenumbruch. Ein am Ende
/// abgeschnittenes Mehrbyte-Zeichen ist erlaubt.
fn looks_like_text(head: &[u8]) -> bool {
    if head.is_empty() {
        return false;
    }
    let valid = match std::str::from_utf8(head) {
        Ok(text) => text,
        Err(err) if err.error_len().is_none() => {
            // Nur das letzte Zeichen ist unvollständig.
            std::str::from_utf8(&head[..err.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return false,
    };
    !valid.is_empty()
        && !valid
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{c}'))
}

/// Ergebnis der Bildbearbeitung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImageInfo {
    /// Anzeigegröße (bei gedrehten JPEGs schon vertauscht).
    pub width: u32,
    pub height: u32,
}

impl ImageInfo {
    /// Klein genug für die Inline-Anzeige?
    pub fn inline_ok(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= MAX_INLINE_SIDE
            && self.height <= MAX_INLINE_SIDE
            && u64::from(self.width) * u64::from(self.height) <= MAX_INLINE_PIXELS
    }
}

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Bild von `input` nach `output` kopieren und dabei Metadaten entfernen.
/// Liefert die Anzeigegröße. Bei kaputten Dateien gibt es einen Fehler; der
/// Aufrufer speichert die Datei dann als gewöhnlichen Download.
pub fn strip_image<R: Read + Seek, W: Write>(
    kind: Kind,
    input: &mut R,
    output: &mut W,
) -> io::Result<ImageInfo> {
    match kind {
        Kind::Jpeg => strip_jpeg(&mut io::BufReader::new(input), output),
        Kind::Png => strip_png(&mut io::BufReader::new(input), output),
        Kind::Webp => strip_webp(input, output),
        Kind::Gif => copy_gif(input, output),
        _ => Err(invalid("not an image")),
    }
}

// --------------------------------------------------------------------- JPEG

/// Segmente ohne Längenfeld.
fn jpeg_standalone(marker: u8) -> bool {
    matches!(marker, 0x01 | 0xD0..=0xD9)
}

fn jpeg_is_sof(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
}

/// Bleibt das Segment erhalten? APP0 (JFIF), APP2 (ICC-Profil) und APP14
/// (Adobe-Farbraum) werden gebraucht, die übrigen APPn und Kommentare nicht.
fn jpeg_keep(marker: u8, payload: &[u8]) -> bool {
    match marker {
        0xE0 | 0xEE => true,
        0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
        0xE1 | 0xE3..=0xED | 0xEF | 0xFE => false,
        _ => true,
    }
}

fn read_u8<R: Read>(input: &mut R) -> io::Result<u8> {
    let mut byte = [0u8; 1];
    input.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn read_u16_be<R: Read>(input: &mut R) -> io::Result<u16> {
    let mut bytes = [0u8; 2];
    input.read_exact(&mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
}

/// Nächsten Marker lesen (Füllbytes `FF FF …` überspringen).
fn jpeg_marker<R: Read>(input: &mut R) -> io::Result<u8> {
    if read_u8(input)? != 0xFF {
        return Err(invalid("jpeg: marker expected"));
    }
    loop {
        let marker = read_u8(input)?;
        if marker != 0xFF {
            return Ok(marker);
        }
    }
}

/// Entropie-kodierte Daten nach einem SOS kopieren, bis der nächste echte
/// Marker kommt (`FF 00` und `FF D0–D7` gehören noch zu den Daten).
fn jpeg_scan<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> io::Result<u8> {
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Err(invalid("jpeg: truncated scan"));
        }
        match buffer.iter().position(|&byte| byte == 0xFF) {
            None => {
                let len = buffer.len();
                output.write_all(buffer)?;
                input.consume(len);
            }
            Some(pos) => {
                output.write_all(&buffer[..pos])?;
                input.consume(pos + 1);
                let mut next = read_u8(input)?;
                while next == 0xFF {
                    next = read_u8(input)?;
                }
                if next == 0x00 || (0xD0..=0xD7).contains(&next) {
                    output.write_all(&[0xFF, next])?;
                } else {
                    return Ok(next);
                }
            }
        }
    }
}

fn strip_jpeg<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> io::Result<ImageInfo> {
    if read_u16_be(input)? != 0xFFD8 {
        return Err(invalid("jpeg: missing SOI"));
    }
    output.write_all(&[0xFF, 0xD8])?;
    let mut size: Option<(u32, u32)> = None;
    let mut orientation: Option<u16> = None;
    let mut wrote_orientation = false;
    let mut pending = None::<u8>;
    loop {
        let marker = match pending.take() {
            Some(marker) => marker,
            None => jpeg_marker(input)?,
        };
        if marker == 0xD9 {
            output.write_all(&[0xFF, 0xD9])?;
            break;
        }
        if jpeg_standalone(marker) {
            output.write_all(&[0xFF, marker])?;
            continue;
        }
        let len = read_u16_be(input)?;
        if len < 2 {
            return Err(invalid("jpeg: bad segment length"));
        }
        let mut payload = vec![0u8; usize::from(len - 2)];
        input.read_exact(&mut payload)?;

        if marker == 0xE1 && payload.starts_with(b"Exif\0\0") && orientation.is_none() {
            orientation = exif_orientation(&payload[6..]);
        }
        if jpeg_is_sof(marker) && payload.len() >= 5 && size.is_none() {
            let height = u32::from(u16::from_be_bytes([payload[1], payload[2]]));
            let width = u32::from(u16::from_be_bytes([payload[3], payload[4]]));
            size = Some((width, height));
        }
        // Die Drehung vor dem ersten Bild-Segment wieder einsetzen.
        let structural = !matches!(marker, 0xE0..=0xEF | 0xFE);
        if structural && !wrote_orientation {
            if let Some(value) = orientation.filter(|value| (2..=8).contains(value)) {
                output.write_all(&minimal_exif(value))?;
            }
            wrote_orientation = true;
        }
        if jpeg_keep(marker, &payload) {
            output.write_all(&[0xFF, marker])?;
            output.write_all(&len.to_be_bytes())?;
            output.write_all(&payload)?;
        }
        if marker == 0xDA {
            pending = Some(jpeg_scan(input, output)?);
        }
    }
    let (width, height) = size.ok_or_else(|| invalid("jpeg: no frame header"))?;
    let rotated = orientation.is_some_and(|value| (5..=8).contains(&value));
    Ok(if rotated {
        ImageInfo {
            width: height,
            height: width,
        }
    } else {
        ImageInfo { width, height }
    })
}

/// Orientierung (Tag 0x0112) aus IFD0 eines TIFF-Blocks lesen.
fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let big = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let bytes: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if big {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let bytes: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    for index in 0..count.min(256) {
        let entry = ifd + 2 + index * 12;
        if u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 {
            return u16_at(entry + 8);
        }
    }
    None
}

/// APP1-Segment mit genau einem Eintrag: der Orientierung.
fn minimal_exif(orientation: u16) -> Vec<u8> {
    let mut segment = Vec::with_capacity(36);
    segment.extend_from_slice(&[0xFF, 0xE1, 0x00, 34]);
    segment.extend_from_slice(b"Exif\0\0");
    segment.extend_from_slice(b"MM\x00\x2A\x00\x00\x00\x08");
    segment.extend_from_slice(&1u16.to_be_bytes());
    segment.extend_from_slice(&0x0112u16.to_be_bytes());
    segment.extend_from_slice(&3u16.to_be_bytes());
    segment.extend_from_slice(&1u32.to_be_bytes());
    segment.extend_from_slice(&orientation.to_be_bytes());
    segment.extend_from_slice(&[0, 0]);
    segment.extend_from_slice(&0u32.to_be_bytes());
    segment
}

// ---------------------------------------------------------------------- PNG

/// Textblöcke, EXIF und Zeitstempel fallen weg; alles andere (auch APNG und
/// Farbprofile) bleibt.
fn png_drop(kind: &[u8; 4]) -> bool {
    matches!(kind, b"eXIf" | b"tEXt" | b"iTXt" | b"zTXt" | b"tIME")
}

fn strip_png<R: Read, W: Write>(input: &mut R, output: &mut W) -> io::Result<ImageInfo> {
    let mut signature = [0u8; 8];
    input.read_exact(&mut signature)?;
    if &signature != b"\x89PNG\r\n\x1a\n" {
        return Err(invalid("png: bad signature"));
    }
    output.write_all(&signature)?;
    let mut info = None;
    loop {
        let mut header = [0u8; 8];
        input.read_exact(&mut header)?;
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        if len > 0x7FFF_FFFF {
            return Err(invalid("png: chunk too large"));
        }
        let kind: [u8; 4] = [header[4], header[5], header[6], header[7]];
        // Daten + CRC
        let rest = u64::from(len) + 4;
        if &kind == b"IHDR" {
            if len != 13 || info.is_some() {
                return Err(invalid("png: bad IHDR"));
            }
            let mut data = vec![0u8; rest as usize];
            input.read_exact(&mut data)?;
            info = Some(ImageInfo {
                width: u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
                height: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            });
            output.write_all(&header)?;
            output.write_all(&data)?;
            continue;
        }
        if info.is_none() {
            return Err(invalid("png: IHDR must come first"));
        }
        if png_drop(&kind) {
            let skipped = io::copy(&mut input.take(rest), &mut io::sink())?;
            if skipped != rest {
                return Err(invalid("png: truncated chunk"));
            }
        } else {
            output.write_all(&header)?;
            let copied = io::copy(&mut input.take(rest), output)?;
            if copied != rest {
                return Err(invalid("png: truncated chunk"));
            }
        }
        if &kind == b"IEND" {
            break;
        }
    }
    info.ok_or_else(|| invalid("png: no IHDR"))
}

// --------------------------------------------------------------------- WebP

const WEBP_FLAG_EXIF: u8 = 0x08;
const WEBP_FLAG_XMP: u8 = 0x04;

struct RiffChunk {
    fourcc: [u8; 4],
    /// Position der Nutzdaten in der Datei.
    offset: u64,
    size: u32,
}

impl RiffChunk {
    /// Größe samt Kopf und Füllbyte.
    fn total(&self) -> u64 {
        8 + u64::from(self.size) + u64::from(self.size & 1)
    }

    fn dropped(&self) -> bool {
        matches!(&self.fourcc, b"EXIF" | b"XMP ")
    }
}

fn strip_webp<R: Read + Seek, W: Write>(input: &mut R, output: &mut W) -> io::Result<ImageInfo> {
    let mut header = [0u8; 12];
    input.read_exact(&mut header)?;
    if &header[..4] != b"RIFF" || &header[8..12] != b"WEBP" {
        return Err(invalid("webp: bad header"));
    }
    let riff_size = u64::from(u32::from_le_bytes([
        header[4], header[5], header[6], header[7],
    ]));
    let end = 8 + riff_size;

    // Erster Durchgang: Chunk-Liste und Größe bestimmen.
    let mut chunks = Vec::new();
    let mut position = 12u64;
    while position + 8 <= end {
        input.seek(SeekFrom::Start(position))?;
        let mut chunk_header = [0u8; 8];
        input.read_exact(&mut chunk_header)?;
        let chunk = RiffChunk {
            fourcc: [
                chunk_header[0],
                chunk_header[1],
                chunk_header[2],
                chunk_header[3],
            ],
            offset: position + 8,
            size: u32::from_le_bytes([
                chunk_header[4],
                chunk_header[5],
                chunk_header[6],
                chunk_header[7],
            ]),
        };
        position += chunk.total();
        if position > end {
            return Err(invalid("webp: chunk exceeds file"));
        }
        chunks.push(chunk);
        if chunks.len() > 100_000 {
            return Err(invalid("webp: too many chunks"));
        }
    }
    let info = webp_size(input, &chunks)?;
    let kept: u64 = chunks
        .iter()
        .filter(|chunk| !chunk.dropped())
        .map(RiffChunk::total)
        .sum();
    let new_riff = u32::try_from(4 + kept).map_err(|_| invalid("webp: too large"))?;

    // Zweiter Durchgang: schreiben.
    output.write_all(b"RIFF")?;
    output.write_all(&new_riff.to_le_bytes())?;
    output.write_all(b"WEBP")?;
    for chunk in chunks.iter().filter(|chunk| !chunk.dropped()) {
        input.seek(SeekFrom::Start(chunk.offset - 8))?;
        let total = chunk.total();
        if &chunk.fourcc == b"VP8X" {
            if chunk.size != 10 {
                return Err(invalid("webp: bad VP8X"));
            }
            let mut data = vec![0u8; total as usize];
            input.read_exact(&mut data)?;
            data[8] &= !(WEBP_FLAG_EXIF | WEBP_FLAG_XMP);
            output.write_all(&data)?;
        } else {
            let copied = io::copy(&mut input.by_ref().take(total), output)?;
            if copied != total {
                return Err(invalid("webp: truncated chunk"));
            }
        }
    }
    Ok(info)
}

fn webp_size<R: Read + Seek>(input: &mut R, chunks: &[RiffChunk]) -> io::Result<ImageInfo> {
    let read_at = |input: &mut R, offset: u64, len: usize| -> io::Result<Vec<u8>> {
        input.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0u8; len];
        input.read_exact(&mut data)?;
        Ok(data)
    };
    let u24 =
        |bytes: &[u8]| u32::from(bytes[0]) | u32::from(bytes[1]) << 8 | u32::from(bytes[2]) << 16;
    for chunk in chunks {
        match &chunk.fourcc {
            b"VP8X" if chunk.size >= 10 => {
                let data = read_at(input, chunk.offset, 10)?;
                return Ok(ImageInfo {
                    width: u24(&data[4..7]) + 1,
                    height: u24(&data[7..10]) + 1,
                });
            }
            b"VP8 " if chunk.size >= 10 => {
                let data = read_at(input, chunk.offset, 10)?;
                if data[3..6] != [0x9D, 0x01, 0x2A] {
                    return Err(invalid("webp: bad VP8 start code"));
                }
                return Ok(ImageInfo {
                    width: u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3FFF),
                    height: u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3FFF),
                });
            }
            b"VP8L" if chunk.size >= 5 => {
                let data = read_at(input, chunk.offset, 5)?;
                if data[0] != 0x2F {
                    return Err(invalid("webp: bad VP8L signature"));
                }
                let bits = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
                return Ok(ImageInfo {
                    width: (bits & 0x3FFF) + 1,
                    height: ((bits >> 14) & 0x3FFF) + 1,
                });
            }
            _ => {}
        }
    }
    Err(invalid("webp: no image data"))
}

// ---------------------------------------------------------------------- GIF

/// GIFs tragen praktisch nie Ortsdaten; sie werden unverändert kopiert, nur
/// die Größe wird gelesen.
fn copy_gif<R: Read + Seek, W: Write>(input: &mut R, output: &mut W) -> io::Result<ImageInfo> {
    let mut head = [0u8; 10];
    input.read_exact(&mut head)?;
    if &head[..6] != b"GIF87a" && &head[..6] != b"GIF89a" {
        return Err(invalid("gif: bad header"));
    }
    let info = ImageInfo {
        width: u32::from(u16::from_le_bytes([head[6], head[7]])),
        height: u32::from(u16::from_le_bytes([head[8], head[9]])),
    };
    input.seek(SeekFrom::Start(0))?;
    io::copy(input, output)?;
    Ok(info)
}

// ----------------------------------------------------------------- Dateinamen

/// Dateinamen für Anzeige und `Content-Disposition` bereinigen: kein Pfad,
/// keine Steuerzeichen, höchstens 200 Zeichen. Leer → „datei“.
pub fn clean_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '<' | '>' | '|' | '*' | '?' | ':'))
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    let mut name: String = trimmed.chars().take(200).collect();
    if name.is_empty() {
        name = "datei".to_owned();
    }
    name
}

/// `Content-Disposition` nach RFC 6266/5987: ASCII-Ersatzname plus UTF-8.
pub fn content_disposition(inline: bool, filename: &str) -> String {
    let ascii: String = filename
        .chars()
        .map(|c| {
            if (c.is_ascii_graphic() && c != '"' && c != '\\') || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::with_capacity(filename.len() * 3);
    for byte in filename.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    let disposition = if inline { "inline" } else { "attachment" };
    format!("{disposition}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn jpeg_with_exif(orientation: u16) -> Vec<u8> {
        let mut data = vec![0xFF, 0xD8];
        // APP0 JFIF
        data.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        data.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        // APP1 EXIF (little endian) mit Orientierung und einem GPS-Verweis
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II\x2A\x00\x08\x00\x00\x00");
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0x8825u16.to_le_bytes()); // GPS-IFD
        tiff.extend_from_slice(&4u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(b"GPS SECRET 52.5200N 13.4050E");
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        data.extend_from_slice(&[0xFF, 0xE1]);
        data.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        data.extend_from_slice(&app1);
        // Kommentar
        data.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x07]);
        data.extend_from_slice(b"hello");
        // SOF0: 8 bit, Höhe 30, Breite 40, 1 Komponente
        data.extend_from_slice(&[
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 30, 0x00, 40, 0x01, 1, 0x11, 0,
        ]);
        // SOS + Daten mit Stuffing und Restart-Marker
        data.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
        data.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]);
        data.extend_from_slice(&[0xFF, 0xD9]);
        // Anhängsel nach dem Ende (Polyglot)
        data.extend_from_slice(b"<script>alert(1)</script>");
        data
    }

    #[test]
    fn sniff_detects_common_types() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Kind::Jpeg);
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Kind::Png);
        assert_eq!(sniff(b"GIF89a.."), Kind::Gif);
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Kind::Webp);
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVEfmt "), Kind::Wav);
        assert_eq!(sniff(b"\0\0\0\x18ftypmp42...."), Kind::Mp4);
        assert_eq!(sniff(b"\0\0\0\x18ftypheic...."), Kind::Other);
        assert_eq!(sniff(b"%PDF-1.7"), Kind::Pdf);
        assert_eq!(sniff(b"hallo welt\n"), Kind::Text);
        assert_eq!(sniff("grüße".as_bytes()), Kind::Text);
        assert_eq!(
            sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            Kind::Text
        );
        assert_eq!(sniff(&[0, 1, 2, 3]), Kind::Other);
        assert_eq!(sniff(&[]), Kind::Other);
        // Abgeschnittenes Mehrbyte-Zeichen am Ende
        assert_eq!(sniff(&"äöü".as_bytes()[..5]), Kind::Text);
    }

    #[test]
    fn svg_is_never_an_image() {
        assert!(!sniff(b"<svg/>").is_image());
    }

    #[test]
    fn jpeg_exif_is_removed_but_orientation_kept() {
        let input = jpeg_with_exif(6);
        let mut output = Vec::new();
        let info = strip_image(Kind::Jpeg, &mut Cursor::new(&input), &mut output).unwrap();
        // Orientierung 6 = 90° gedreht → Breite und Höhe vertauscht
        assert_eq!(
            info,
            ImageInfo {
                width: 30,
                height: 40
            }
        );
        let text = String::from_utf8_lossy(&output);
        assert!(!text.contains("GPS SECRET"));
        assert!(!text.contains("hello"));
        assert!(!text.contains("script"));
        assert!(text.contains("JFIF"));
        // Der neue EXIF-Block trägt nur noch die Orientierung.
        let app1 = output.windows(6).position(|w| w == b"Exif\0\0").unwrap();
        assert_eq!(exif_orientation(&output[app1 + 6..]), Some(6));
        // Scan-Daten unverändert, Ende intakt
        assert!(
            output
                .windows(7)
                .any(|w| w == [0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56])
        );
        assert_eq!(&output[output.len() - 2..], &[0xFF, 0xD9]);
        // Zweiter Durchlauf ist stabil
        let mut again = Vec::new();
        strip_image(Kind::Jpeg, &mut Cursor::new(&output), &mut again).unwrap();
        assert_eq!(again, output);
    }

    #[test]
    fn jpeg_without_rotation_gets_no_exif() {
        let input = jpeg_with_exif(1);
        let mut output = Vec::new();
        let info = strip_image(Kind::Jpeg, &mut Cursor::new(&input), &mut output).unwrap();
        assert_eq!((info.width, info.height), (40, 30));
        assert!(!output.windows(4).any(|w| w == b"Exif"));
    }

    #[test]
    fn truncated_jpeg_is_an_error() {
        let input = jpeg_with_exif(1);
        let mut output = Vec::new();
        assert!(strip_image(Kind::Jpeg, &mut Cursor::new(&input[..60]), &mut output).is_err());
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]); // CRC wird nicht geprüft
        chunk
    }

    #[test]
    fn png_text_chunks_are_removed() {
        let mut input = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = 640u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&480u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        input.extend(png_chunk(b"IHDR", &ihdr));
        input.extend(png_chunk(b"tEXt", b"Comment\0secret place"));
        input.extend(png_chunk(b"eXIf", b"MM\0*gps"));
        input.extend(png_chunk(b"iCCP", b"icc"));
        input.extend(png_chunk(b"IDAT", b"pixels"));
        input.extend(png_chunk(b"IEND", b""));
        input.extend_from_slice(b"trailing polyglot");
        let mut output = Vec::new();
        let info = strip_image(Kind::Png, &mut Cursor::new(&input), &mut output).unwrap();
        assert_eq!((info.width, info.height), (640, 480));
        let text = String::from_utf8_lossy(&output);
        assert!(!text.contains("secret"));
        assert!(!text.contains("gps"));
        assert!(!text.contains("polyglot"));
        assert!(text.contains("iCCP"));
        assert!(text.contains("pixels"));
        assert!(text.ends_with("IEND\0\0\0\0"));
    }

    fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = b"WEBP".to_vec();
        for (fourcc, data) in chunks {
            body.extend_from_slice(*fourcc);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(data);
            if data.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&body);
        file
    }

    #[test]
    fn webp_exif_and_xmp_are_removed() {
        let mut vp8x = vec![WEBP_FLAG_EXIF | WEBP_FLAG_XMP | 0x10, 0, 0, 0];
        vp8x.extend_from_slice(&[0x7F, 0x02, 0x00]); // Breite 640
        vp8x.extend_from_slice(&[0xDF, 0x01, 0x00]); // Höhe 480
        let input = riff(&[
            (b"VP8X", vp8x),
            (b"VP8L", vec![0x2F, 0, 0, 0, 0, 1, 2]),
            (b"EXIF", b"gps secret".to_vec()),
            (b"XMP ", b"<x:xmpmeta secret/>".to_vec()),
        ]);
        let mut output = Vec::new();
        let info = strip_image(Kind::Webp, &mut Cursor::new(&input), &mut output).unwrap();
        assert_eq!((info.width, info.height), (640, 480));
        assert!(!String::from_utf8_lossy(&output).contains("secret"));
        assert_eq!(output[20] & (WEBP_FLAG_EXIF | WEBP_FLAG_XMP), 0);
        assert_eq!(output[20] & 0x10, 0x10, "Alpha-Flag bleibt");
        let riff_size = u32::from_le_bytes([output[4], output[5], output[6], output[7]]);
        assert_eq!(riff_size as usize, output.len() - 8);
        assert_eq!(sniff(&output), Kind::Webp);
    }

    #[test]
    fn webp_simple_lossless_size() {
        // 100 x 50: (w-1) = 99, (h-1) = 49 → bits = 99 | 49 << 14
        let bits: u32 = 99 | (49 << 14);
        let mut vp8l = vec![0x2F];
        vp8l.extend_from_slice(&bits.to_le_bytes());
        let input = riff(&[(b"VP8L", vp8l)]);
        let mut output = Vec::new();
        let info = strip_image(Kind::Webp, &mut Cursor::new(&input), &mut output).unwrap();
        assert_eq!((info.width, info.height), (100, 50));
        assert_eq!(output, input);
    }

    #[test]
    fn gif_size_is_read() {
        let mut input = b"GIF89a".to_vec();
        input.extend_from_slice(&320u16.to_le_bytes());
        input.extend_from_slice(&200u16.to_le_bytes());
        input.extend_from_slice(b"rest");
        let mut output = Vec::new();
        let info = strip_image(Kind::Gif, &mut Cursor::new(&input), &mut output).unwrap();
        assert_eq!((info.width, info.height), (320, 200));
        assert_eq!(output, input);
    }

    #[test]
    fn pixel_bombs_are_not_inline() {
        assert!(
            ImageInfo {
                width: 4000,
                height: 3000
            }
            .inline_ok()
        );
        assert!(
            !ImageInfo {
                width: 30_000,
                height: 30_000
            }
            .inline_ok()
        );
        assert!(
            !ImageInfo {
                width: 0,
                height: 10
            }
            .inline_ok()
        );
    }

    #[test]
    fn filenames_are_cleaned() {
        assert_eq!(clean_filename("../../etc/passwd"), "passwd");
        assert_eq!(clean_filename("C:\\Users\\x\\bild.png"), "bild.png");
        assert_eq!(clean_filename(".htaccess"), "htaccess");
        assert_eq!(clean_filename("a\u{0}b\nc.txt"), "abc.txt");
        assert_eq!(clean_filename(""), "datei");
        assert_eq!(clean_filename("   "), "datei");
        assert_eq!(clean_filename(&"x".repeat(300)).chars().count(), 200);
    }

    #[test]
    fn disposition_encodes_utf8() {
        let header = content_disposition(false, "Grüße \"x\".pdf");
        assert_eq!(
            header,
            "attachment; filename=\"Gr__e _x_.pdf\"; filename*=UTF-8''Gr%C3%BC%C3%9Fe%20%22x%22.pdf"
        );
        assert!(content_disposition(true, "a.png").starts_with("inline;"));
    }
}
