//! H.264 für Streams über Ciscos OpenH264 (aus dem Quelltext gebaut).
//!
//! Der SFU bietet für Video nur H.264 Constrained Baseline an
//! (`crates/voice-sfu`, `rtc_config`); Browser und nativer Client sprechen so
//! denselben Codec. str0m liefert ankommende Bilder bereits zusammengesetzt
//! als Annex-B-Bytestrom, ausgehende Bilder werden ebenso übergeben.

use openh264::OpenH264API;
use openh264::decoder::Decoder;
use openh264::encoder::{
    BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode,
    UsageType,
};
use openh264::formats::{BgraSliceU8, YUVBuffer, YUVSource};

use crate::capture;

/// Ziel-Bildrate beim Teilen.
pub const FPS: u32 = 30;
/// Bitrate beim Teilen; Bildschirminhalt mit Text braucht mehr als Kamera.
const BITRATE: u32 = 3_000_000;

/// Ein dekodiertes Bild (RGBA, Zeilen ohne Lücken).
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct H264Encoder {
    inner: Encoder,
}

impl H264Encoder {
    pub fn new() -> Result<Self, String> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .profile(Profile::Baseline)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(BITRATE))
            .max_frame_rate(FrameRate::from_hz(FPS as f32))
            // Spätestens alle 4 s ein Schlüsselbild: wer neu dazukommt oder
            // Pakete verliert, sieht schnell wieder ein Bild.
            .intra_frame_period(IntraFramePeriod::from_num_frames(FPS * 4))
            .skip_frames(true);
        let inner = Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|err| format!("H.264-Kodierer: {err}"))?;
        Ok(Self { inner })
    }

    /// Ein Bild kodieren; `None`, wenn der Kodierer es (Bitratenbudget)
    /// übersprungen hat.
    pub fn encode(&mut self, frame: &capture::Frame) -> Result<Option<Vec<u8>>, String> {
        let yuv = YUVBuffer::from_bgra8_source(BgraSliceU8::new(
            &frame.bgra,
            (frame.width, frame.height),
        ));
        let stream = self
            .inner
            .encode(&yuv)
            .map_err(|err| format!("Kodieren fehlgeschlagen: {err}"))?;
        let bytes = stream.to_vec();
        Ok((!bytes.is_empty()).then_some(bytes))
    }

    /// Nächstes Bild als Schlüsselbild (auf Wunsch eines Empfängers).
    pub fn force_keyframe(&mut self) {
        self.inner.force_intra_frame();
    }
}

pub struct H264Decoder {
    inner: Decoder,
}

impl H264Decoder {
    pub fn new() -> Result<Self, String> {
        Decoder::new()
            .map(|inner| Self { inner })
            .map_err(|err| format!("H.264-Dekodierer: {err}"))
    }

    /// Einen Annex-B-Bytestrom (ein Bild) dekodieren. `Ok(None)`: noch kein
    /// vollständiges Bild (z. B. nur SPS/PPS); `Err`: Schlüsselbild anfordern.
    pub fn decode(&mut self, annex_b: &[u8]) -> Result<Option<VideoFrame>, String> {
        let Some(yuv) = self
            .inner
            .decode(annex_b)
            .map_err(|err| format!("Dekodieren fehlgeschlagen: {err}"))?
        else {
            return Ok(None);
        };
        let (width, height) = yuv.dimensions();
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let mut rgba = vec![0u8; width * height * 4];
        yuv.write_rgba8(&mut rgba);
        Ok(Some(VideoFrame {
            width: width as u32,
            height: height as u32,
            rgba,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Testbild: Farbverlauf, damit der Kodierer echte Daten sieht.
    fn gradient(width: usize, height: usize, shift: u8) -> capture::Frame {
        let mut bgra = vec![0u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let i = (y * width + x) * 4;
                bgra[i] = (x as u8).wrapping_add(shift);
                bgra[i + 1] = y as u8;
                bgra[i + 2] = 128;
                bgra[i + 3] = 255;
            }
        }
        capture::Frame {
            width,
            height,
            bgra,
        }
    }

    #[test]
    fn encode_then_decode_roundtrip() {
        let mut encoder = H264Encoder::new().expect("Kodierer");
        let mut decoder = H264Decoder::new().expect("Dekodierer");
        let mut decoded = None;
        for shift in 0..5u8 {
            let frame = gradient(320, 240, shift * 10);
            if let Some(bytes) = encoder.encode(&frame).expect("kodieren") {
                // Annex B beginnt mit einem Startcode.
                assert!(bytes.starts_with(&[0, 0, 0, 1]) || bytes.starts_with(&[0, 0, 1]));
                if let Some(frame) = decoder.decode(&bytes).expect("dekodieren") {
                    decoded = Some(frame);
                }
            }
        }
        let frame = decoded.expect("mindestens ein Bild dekodiert");
        assert_eq!((frame.width, frame.height), (320, 240));
        assert_eq!(frame.rgba.len(), 320 * 240 * 4);
        // Grünkanal folgt der Zeile: unten heller als oben (verlustbehaftet).
        let top = frame.rgba[1];
        let bottom = frame.rgba[(239 * 320) * 4 + 1];
        assert!(
            bottom > top.saturating_add(100),
            "oben {top}, unten {bottom}"
        );
    }
}

#[cfg(test)]
mod screen_tests {
    use super::*;

    /// Echter Bildschirm: nur manuell (`cargo test -- --ignored`), die CI hat keinen.
    #[test]
    #[ignore = "braucht einen echten Bildschirm"]
    fn grab_and_encode_screen() {
        let mut capturer = capture::open().expect("Aufnahme öffnen");
        let mut encoder = H264Encoder::new().expect("Kodierer");
        let (mut grab, mut encode) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let mut bytes = 0;
        let mut size = (0, 0);
        for _ in 0..20 {
            let t = std::time::Instant::now();
            let frame = capturer.grab().expect("Bild aufnehmen");
            grab += t.elapsed();
            assert!(frame.width > 0 && frame.height > 0);
            size = (frame.width, frame.height);
            let t = std::time::Instant::now();
            bytes += encoder
                .encode(&frame)
                .expect("kodieren")
                .map_or(0, |b| b.len());
            encode += t.elapsed();
        }
        eprintln!(
            "{size:?}: Aufnahme {:?}, Kodierung {:?} je Bild, {bytes} Bytes für 20 Bilder",
            grab / 20,
            encode / 20
        );
        assert!(bytes > 0);
    }
}
