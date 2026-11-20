//! Ende-zu-Ende-Latenz mit echten Geräten (Mund → Ohr), ohne Uhrenabgleich.
//!
//! 1. `--mode clicktrack --out click.wav`: Klick-Spur erzeugen (48 kHz mono,
//!    alle 1000 ms ein 2-ms-Klick).
//! 2. Die Spur neben dem Mikrofon von Gerät A abspielen. Ein Stereo-Rekorder
//!    nimmt links die Klickquelle auf (Referenz) und rechts den Lautsprecher
//!    oder Kopfhörer von Gerät B, das im selben Sprachkanal ist.
//! 3. `--mode clickdelay --wav aufnahme.wav`: findet jeden Klick links, sucht
//!    ihn rechts im Fenster bis 1 s danach und gibt die Verzögerungen aus.
//!
//! WAV wird von Hand gelesen und geschrieben (PCM 16 Bit oder Float 32 Bit),
//! damit das Werkzeug ohne zusätzliche Abhängigkeit auskommt.

use std::{fs, path::Path};

use anyhow::{Context, Result, anyhow, bail};

const RATE: u32 = 48_000;
/// Abstand der Klicks; größer als jede erwartete Latenz.
const PERIOD_MS: u32 = 1000;
/// Nach einem Klick so lange keinen neuen suchen.
const REFRACTORY_MS: u32 = 500;
/// Schwelle relativ zum lautesten Wert des Kanals.
const THRESHOLD: f32 = 0.35;

/// Klick-Spur schreiben: `seconds` lang, ein kurzer 2-kHz-Burst pro Sekunde.
pub fn write_track(path: &Path, seconds: u32) -> Result<()> {
    let total = (RATE * seconds) as usize;
    let mut samples = vec![0i16; total];
    let period = (RATE * PERIOD_MS / 1000) as usize;
    let burst = (RATE / 500) as usize; // 2 ms
    for start in (period / 2..total).step_by(period) {
        for i in 0..burst.min(total - start) {
            let phase = i as f32 / RATE as f32 * 2000.0 * std::f32::consts::TAU;
            // Hann-Fenster: steiler Anstieg, aber kein Knacken außerhalb.
            let window = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / burst as f32).cos();
            samples[start + i] = (phase.sin() * window * 0.9 * f32::from(i16::MAX)) as i16;
        }
    }
    let mut data = Vec::with_capacity(total * 2);
    for sample in samples {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, wav_bytes(1, RATE, 16, 1, &data))
        .with_context(|| format!("{} schreiben", path.display()))?;
    println!(
        "Klick-Spur: {} ({seconds} s, ein Klick pro Sekunde, 48 kHz mono)",
        path.display()
    );
    Ok(())
}

/// Stereo-Aufnahme auswerten und Verzögerungen ausgeben.
pub fn analyze(path: &Path) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("{} lesen", path.display()))?;
    let wav = Wav::parse(&bytes)?;
    if wav.channels < 2 {
        bail!("Stereo-Aufnahme nötig: links Referenz, rechts Empfänger");
    }
    let left = wav.channel(0);
    let right = wav.channel(1);
    let delays = delays(&left, &right, wav.rate);
    if delays.is_empty() {
        bail!("keine Klicks gefunden – Pegel prüfen");
    }
    let found = delays.iter().filter(|d| d.is_some()).count();
    let mut ms: Vec<f64> = delays.iter().flatten().copied().collect();
    ms.sort_by(f64::total_cmp);
    let pick = |q: f64| ms[((ms.len() as f64 * q) as usize).min(ms.len() - 1)];
    println!(
        "{} Klicks, {found} beim Empfänger gefunden ({} fehlen)",
        delays.len(),
        delays.len() - found
    );
    if !ms.is_empty() {
        println!(
            "Ende-zu-Ende: p50 {:.1} ms · p95 {:.1} ms · min {:.1} ms · max {:.1} ms",
            pick(0.50),
            pick(0.95),
            ms[0],
            ms[ms.len() - 1]
        );
    }
    Ok(())
}

/// Pro Klick in `reference` die Verzögerung bis zum Klick in `received` (ms);
/// `None`, wenn dort im Fenster nichts ankam.
fn delays(reference: &[f32], received: &[f32], rate: u32) -> Vec<Option<f64>> {
    let window = (rate * PERIOD_MS / 1000) as usize;
    let threshold =
        |samples: &[f32]| samples.iter().fold(0.0f32, |max, s| max.max(s.abs())) * THRESHOLD;
    let (ref_level, rx_level) = (threshold(reference), threshold(received));
    if ref_level <= 0.0 {
        return Vec::new();
    }
    let onsets = onsets(reference, ref_level, rate);
    onsets
        .into_iter()
        .map(|start| {
            if rx_level <= 0.0 {
                return None;
            }
            let end = (start + window).min(received.len());
            received[start.min(end)..end]
                .iter()
                .position(|sample| sample.abs() >= rx_level)
                .map(|offset| offset as f64 * 1000.0 / f64::from(rate))
        })
        .collect()
}

fn onsets(samples: &[f32], level: f32, rate: u32) -> Vec<usize> {
    let refractory = (rate * REFRACTORY_MS / 1000) as usize;
    let mut found = Vec::new();
    let mut index = 0;
    while index < samples.len() {
        if samples[index].abs() >= level {
            found.push(index);
            index += refractory;
        } else {
            index += 1;
        }
    }
    found
}

struct Wav {
    channels: u16,
    rate: u32,
    /// Verschachtelte Samples, auf −1…1 normiert.
    samples: Vec<f32>,
}

impl Wav {
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            bail!("keine WAV-Datei");
        }
        let mut pos = 12;
        let mut format: Option<(u16, u16, u32, u16)> = None;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into()?) as usize;
            let body = bytes
                .get(pos + 8..pos + 8 + len)
                .ok_or_else(|| anyhow!("WAV abgeschnitten"))?;
            match id {
                b"fmt " if len >= 16 => {
                    let tag = u16::from_le_bytes(body[0..2].try_into()?);
                    let channels = u16::from_le_bytes(body[2..4].try_into()?);
                    let rate = u32::from_le_bytes(body[4..8].try_into()?);
                    let bits = u16::from_le_bytes(body[14..16].try_into()?);
                    // 0xFFFE (Extensible): Untertyp steht weiter hinten.
                    let tag = if tag == 0xFFFE && len >= 26 {
                        u16::from_le_bytes(body[24..26].try_into()?)
                    } else {
                        tag
                    };
                    format = Some((tag, channels, rate, bits));
                }
                b"data" => {
                    let (tag, channels, rate, bits) =
                        format.ok_or_else(|| anyhow!("fmt-Block fehlt"))?;
                    let samples: Vec<f32> = match (tag, bits) {
                        (1, 16) => body
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|c| f32::from(i16::from_le_bytes(*c)) / 32768.0)
                            .collect(),
                        (3, 32) => body
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|c| f32::from_le_bytes(*c))
                            .collect(),
                        _ => bail!("nur PCM 16 Bit oder Float 32 Bit (Format {tag}, {bits} Bit)"),
                    };
                    return Ok(Self {
                        channels: channels.max(1),
                        rate,
                        samples,
                    });
                }
                _ => {}
            }
            // Blöcke sind auf gerade Längen aufgefüllt.
            pos += 8 + len + (len & 1);
        }
        bail!("data-Block fehlt")
    }

    fn channel(&self, index: usize) -> Vec<f32> {
        self.samples
            .iter()
            .skip(index)
            .step_by(usize::from(self.channels))
            .copied()
            .collect()
    }
}

fn wav_bytes(format: u16, rate: u32, bits: u16, channels: u16, data: &[u8]) -> Vec<u8> {
    let block = channels * bits / 8;
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stereo-Aufnahme simulieren: rechts dieselben Klicks 73 ms später,
    /// leiser und mit Rauschen; ein Klick geht verloren.
    #[test]
    fn finds_the_delay_of_each_click() {
        let rate = RATE;
        let seconds = 5;
        let total = (rate * seconds) as usize;
        let delay = (rate as usize * 73) / 1000;
        let mut left = vec![0.0f32; total];
        let mut right = vec![0.0f32; total];
        let mut noise = 12345u32;
        for sample in right.iter_mut() {
            noise = noise.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *sample = ((noise >> 16) as f32 / 65536.0 - 0.5) * 0.02;
        }
        for (n, start) in (rate as usize / 2..total)
            .step_by(rate as usize)
            .enumerate()
        {
            for i in 0..96 {
                let value = ((i as f32) * 0.26).sin() * 0.8;
                left[start + i] = value;
                if n != 2 && start + delay + i < total {
                    right[start + delay + i] += value * 0.3;
                }
            }
        }
        let found = delays(&left, &right, rate);
        assert_eq!(found.len(), 5);
        assert_eq!(found[2], None, "verlorener Klick");
        for value in found.iter().flatten() {
            assert!((value - 73.0).abs() < 1.0, "{value}");
        }
    }

    #[test]
    fn wav_roundtrip() {
        let data: Vec<u8> = [0i16, 1000, -1000, 32767]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let bytes = wav_bytes(1, 48_000, 16, 2, &data);
        let wav = Wav::parse(&bytes).expect("wav");
        assert_eq!(wav.channels, 2);
        assert_eq!(wav.rate, 48_000);
        assert_eq!(wav.channel(1).len(), 2);
        assert!((wav.channel(1)[1] - 32767.0 / 32768.0).abs() < 1e-6);
    }
}
