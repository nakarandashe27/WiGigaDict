use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use webrtc_vad::{SampleRate, Vad, VadMode};

const SAMPLE_RATE: u32 = 16_000;
const TAIL_SAMPLES: usize = 16_000;
const FRAME_SAMPLES: usize = 320;
const FRAME_MS: u32 = 20;
const LEGACY_PEAK_DBFS: f64 = -45.0;
const ENERGY_DBFS: f64 = -50.0;
const MIN_TOTAL_MS: u32 = 100;
const MIN_CONSECUTIVE_MS: u32 = 60;

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    schema_version: u32,
    #[serde(default = "default_purpose")]
    purpose: String,
    samples: Vec<FixtureSample>,
}

fn default_purpose() -> String {
    "fixture".to_string()
}

#[derive(Debug, Deserialize)]
struct FixtureSample {
    id: String,
    split: String,
    label: String,
    subtype: String,
    path: String,
    sha256: String,
}

#[derive(Debug)]
struct WavPcm {
    samples: Vec<i16>,
    duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct ContractReport {
    tail_ms: u32,
    frame_ms: u32,
    legacy_peak_dbfs_strictly_above: f64,
    energy_rms_dbfs_strictly_above: f64,
    vad: &'static str,
    min_total_ms: u32,
    min_consecutive_ms: u32,
}

#[derive(Debug, Serialize)]
struct Metrics {
    peak_dbfs: f64,
    energy_active_ms: u32,
    energy_max_consecutive_ms: u32,
    vad_active_ms: u32,
    vad_max_consecutive_ms: u32,
    legacy_peak_pass: bool,
    primary_pass: bool,
}

#[derive(Debug, Serialize)]
struct SampleReport {
    id: String,
    split: String,
    label: String,
    subtype: String,
    relative_path: String,
    sha256: String,
    duration_ms: u64,
    metrics: Metrics,
    legacy_correct: Option<bool>,
    primary_correct: Option<bool>,
}

#[derive(Debug, Default, Serialize)]
struct ClassifierSummary {
    true_positive: u32,
    true_negative: u32,
    false_positive: u32,
    false_negative: u32,
}

#[derive(Debug, Default, Serialize)]
struct SplitSummary {
    samples: u32,
    speech: u32,
    non_speech: u32,
    legacy_peak: ClassifierSummary,
    primary: ClassifierSummary,
}

#[derive(Debug, Serialize)]
struct BoundaryReport {
    schema_version: u32,
    validator: &'static str,
    fixture_manifest_sha256: String,
    fixture_schema_version: u32,
    purpose: String,
    contract: ContractReport,
    acceptance_pass: Option<bool>,
    summaries: BTreeMap<String, SplitSummary>,
    samples: Vec<SampleReport>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("boundary-validator: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (manifest_path, output_path) = parse_args()?;
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let manifest: FixtureManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid manifest {}: {error}", manifest_path.display()))?;
    if manifest.samples.is_empty() {
        return Err("fixture manifest contains no samples".to_string());
    }
    let diagnostic = manifest.purpose == "diagnostic";

    let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut reports = Vec::with_capacity(manifest.samples.len());
    let mut summaries: BTreeMap<String, SplitSummary> = BTreeMap::new();

    for fixture in manifest.samples {
        validate_fixture_metadata(&fixture, diagnostic)?;
        let wav_path = base.join(&fixture.path);
        let wav_bytes = fs::read(&wav_path)
            .map_err(|error| format!("cannot read {}: {error}", wav_path.display()))?;
        let actual_hash = sha256_hex(&wav_bytes);
        if !actual_hash.eq_ignore_ascii_case(&fixture.sha256) {
            return Err(format!(
                "hash mismatch for {}: expected {}, got {}",
                fixture.id, fixture.sha256, actual_hash
            ));
        }
        let wav = parse_wav(&wav_bytes).map_err(|error| format!("{}: {error}", fixture.id))?;
        let metrics =
            analyze_tail(&wav.samples).map_err(|error| format!("{}: {error}", fixture.id))?;
        let expected = match fixture.label.as_str() {
            "speech" => Some(true),
            "non_speech" => Some(false),
            "unknown" if diagnostic => None,
            _ => unreachable!("metadata validation rejects unsupported labels"),
        };
        let legacy_correct = expected.map(|value| metrics.legacy_peak_pass == value);
        let primary_correct = expected.map(|value| metrics.primary_pass == value);

        let summary = summaries.entry(fixture.split.clone()).or_default();
        summary.samples += 1;
        if let Some(expected) = expected {
            if expected {
                summary.speech += 1;
            } else {
                summary.non_speech += 1;
            }
            add_outcome(&mut summary.legacy_peak, expected, metrics.legacy_peak_pass);
            add_outcome(&mut summary.primary, expected, metrics.primary_pass);
        }

        reports.push(SampleReport {
            id: fixture.id,
            split: fixture.split,
            label: fixture.label,
            subtype: fixture.subtype,
            relative_path: fixture.path,
            sha256: actual_hash,
            duration_ms: wav.duration_ms,
            metrics,
            legacy_correct,
            primary_correct,
        });
    }

    let acceptance_pass = if diagnostic {
        None
    } else {
        for required in ["calibration", "heldout"] {
            let summary = summaries
                .get(required)
                .ok_or_else(|| format!("missing required split {required}"))?;
            if summary.speech == 0 || summary.non_speech == 0 {
                return Err(format!("split {required} must contain both labels"));
            }
        }
        let heldout = &summaries["heldout"].primary;
        Some(heldout.false_positive == 0 && heldout.false_negative == 0)
    };
    let report = BoundaryReport {
        schema_version: 1,
        validator: "wigigadict-asr-boundary-validator/0.0.1",
        fixture_manifest_sha256: sha256_hex(&manifest_bytes),
        fixture_schema_version: manifest.schema_version,
        purpose: manifest.purpose,
        contract: ContractReport {
            tail_ms: 1_000,
            frame_ms: FRAME_MS,
            legacy_peak_dbfs_strictly_above: LEGACY_PEAK_DBFS,
            energy_rms_dbfs_strictly_above: ENERGY_DBFS,
            vad: "webrtc-vad 0.4.0; 16 kHz; Aggressive",
            min_total_ms: MIN_TOTAL_MS,
            min_consecutive_ms: MIN_CONSECUTIVE_MS,
        },
        acceptance_pass,
        summaries,
        samples: reports,
    };

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("cannot serialize report: {error}"))?;
    fs::write(&output_path, json)
        .map_err(|error| format!("cannot write {}: {error}", output_path.display()))?;

    println!(
        "boundary analysis: acceptance_pass={acceptance_pass:?} report={}",
        output_path.display()
    );
    if acceptance_pass == Some(false) {
        std::process::exit(2);
    }
    Ok(())
}

fn parse_args() -> Result<(PathBuf, PathBuf), String> {
    let mut manifest = None;
    let mut output = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => manifest = args.next().map(PathBuf::from),
            "--output" => output = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                println!(
                    "Usage: asr-boundary-validator --manifest <fixture.json> --output <report.json>"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok((
        manifest.ok_or_else(|| "missing --manifest".to_string())?,
        output.ok_or_else(|| "missing --output".to_string())?,
    ))
}

fn validate_fixture_metadata(fixture: &FixtureSample, diagnostic: bool) -> Result<(), String> {
    if fixture.id.trim().is_empty() || fixture.subtype.trim().is_empty() {
        return Err("fixture id and subtype must be non-empty".to_string());
    }
    let valid_split = if diagnostic {
        fixture.split == "diagnostic"
    } else {
        fixture.split == "calibration" || fixture.split == "heldout"
    };
    if !valid_split {
        return Err(format!(
            "{} has invalid split {}",
            fixture.id, fixture.split
        ));
    }
    let valid_label = if diagnostic {
        fixture.label == "unknown"
    } else {
        fixture.label == "speech" || fixture.label == "non_speech"
    };
    if !valid_label {
        return Err(format!(
            "{} has invalid label {}",
            fixture.id, fixture.label
        ));
    }
    let path = Path::new(&fixture.path);
    if path.is_absolute() || fixture.path.contains("..") {
        return Err(format!(
            "{} path must be relative and cannot contain ..",
            fixture.id
        ));
    }
    if fixture.sha256.len() != 64 || !fixture.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{} has invalid SHA-256", fixture.id));
    }
    Ok(())
}

fn add_outcome(summary: &mut ClassifierSummary, expected: bool, actual: bool) {
    match (expected, actual) {
        (true, true) => summary.true_positive += 1,
        (false, false) => summary.true_negative += 1,
        (false, true) => summary.false_positive += 1,
        (true, false) => summary.false_negative += 1,
    }
}

fn analyze_tail(samples: &[i16]) -> Result<Metrics, String> {
    if samples.len() < TAIL_SAMPLES {
        return Err(format!(
            "WAV is shorter than 1.0 second: {} samples",
            samples.len()
        ));
    }
    let tail = &samples[samples.len() - TAIL_SAMPLES..];
    let peak = tail
        .iter()
        .map(|sample| i32::from(*sample).unsigned_abs())
        .max()
        .unwrap_or(0) as f64;
    let peak_dbfs = dbfs(peak / 32_768.0);

    let mut energy_flags = Vec::with_capacity(TAIL_SAMPLES / FRAME_SAMPLES);
    let mut vad_flags = Vec::with_capacity(TAIL_SAMPLES / FRAME_SAMPLES);
    let mut vad = Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Aggressive);

    for frame in tail.chunks_exact(FRAME_SAMPLES) {
        let sum_squares: f64 = frame
            .iter()
            .map(|sample| {
                let normalized = f64::from(*sample) / 32_768.0;
                normalized * normalized
            })
            .sum();
        let rms = (sum_squares / FRAME_SAMPLES as f64).sqrt();
        energy_flags.push(dbfs(rms) > ENERGY_DBFS);
        vad_flags.push(
            vad.is_voice_segment(frame)
                .map_err(|_| "WebRTC VAD rejected a 20 ms frame".to_string())?,
        );
    }

    let (energy_active_ms, energy_max_consecutive_ms) = durations(&energy_flags);
    let (vad_active_ms, vad_max_consecutive_ms) = durations(&vad_flags);
    let primary_pass = energy_active_ms >= MIN_TOTAL_MS
        && energy_max_consecutive_ms >= MIN_CONSECUTIVE_MS
        && vad_active_ms >= MIN_TOTAL_MS
        && vad_max_consecutive_ms >= MIN_CONSECUTIVE_MS;

    Ok(Metrics {
        peak_dbfs: round3(peak_dbfs),
        energy_active_ms,
        energy_max_consecutive_ms,
        vad_active_ms,
        vad_max_consecutive_ms,
        legacy_peak_pass: peak_dbfs > LEGACY_PEAK_DBFS,
        primary_pass,
    })
}

fn durations(flags: &[bool]) -> (u32, u32) {
    let mut active_frames = 0_u32;
    let mut consecutive = 0_u32;
    let mut max_consecutive = 0_u32;
    for active in flags {
        if *active {
            active_frames += 1;
            consecutive += 1;
            max_consecutive = max_consecutive.max(consecutive);
        } else {
            consecutive = 0;
        }
    }
    (active_frames * FRAME_MS, max_consecutive * FRAME_MS)
}

fn dbfs(amplitude: f64) -> f64 {
    if amplitude <= 0.0 {
        -120.0
    } else {
        20.0 * amplitude.log10()
    }
}

fn round3(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

fn parse_wav(bytes: &[u8]) -> Result<WavPcm, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_string());
    }
    let mut position = 12_usize;
    let mut format = None;
    let mut data = None;
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = read_u32(bytes, position + 4)? as usize;
        let start = position + 8;
        let end = start
            .checked_add(size)
            .ok_or_else(|| "WAV chunk size overflow".to_string())?;
        if end > bytes.len() {
            return Err("WAV chunk extends beyond file".to_string());
        }
        if id == b"fmt " {
            if size < 16 {
                return Err("WAV fmt chunk is shorter than 16 bytes".to_string());
            }
            format = Some((
                read_u16(bytes, start)?,
                read_u16(bytes, start + 2)?,
                read_u32(bytes, start + 4)?,
                read_u32(bytes, start + 8)?,
                read_u16(bytes, start + 12)?,
                read_u16(bytes, start + 14)?,
            ));
        } else if id == b"data" {
            data = Some(&bytes[start..end]);
        }
        position = end + (size & 1);
    }

    let (audio_format, channels, sample_rate, byte_rate, block_align, bits_per_sample) =
        format.ok_or_else(|| "WAV fmt chunk is missing".to_string())?;
    if audio_format != 1 {
        return Err(format!(
            "WAV must be integer PCM, got format {audio_format}"
        ));
    }
    if channels != 1 || sample_rate != SAMPLE_RATE || bits_per_sample != 16 {
        return Err(format!(
            "WAV must be mono PCM16 at 16000 Hz, got channels={channels} rate={sample_rate} bits={bits_per_sample}"
        ));
    }
    if block_align != 2 || byte_rate != SAMPLE_RATE * 2 {
        return Err(format!(
            "inconsistent WAV byte rate/block align: byte_rate={byte_rate} block_align={block_align}"
        ));
    }
    let data = data.ok_or_else(|| "WAV data chunk is missing".to_string())?;
    if data.len() % 2 != 0 {
        return Err("WAV PCM data has an odd byte count".to_string());
    }
    let samples = data
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    let duration_ms = samples.len() as u64 * 1_000 / u64::from(SAMPLE_RATE);
    Ok(WavPcm {
        samples,
        duration_ms,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "unexpected end of WAV".to_string())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "unexpected end of WAV".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_negative() {
        let metrics = analyze_tail(&vec![0; TAIL_SAMPLES]).expect("silence must analyze");
        assert_eq!(metrics.peak_dbfs, -120.0);
        assert!(!metrics.legacy_peak_pass);
        assert!(!metrics.primary_pass);
    }

    #[test]
    fn isolated_borderline_peak_is_not_speech() {
        let mut samples = vec![0; TAIL_SAMPLES];
        samples[TAIL_SAMPLES / 2] = 260;
        let metrics = analyze_tail(&samples).expect("impulse must analyze");
        assert!(metrics.legacy_peak_pass);
        assert!(!metrics.primary_pass);
        assert_eq!(metrics.energy_active_ms, 0);
    }

    #[test]
    fn wav_parser_accepts_extra_odd_sized_chunk() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"JUNK");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&[7, 0]);
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&123_i16.to_le_bytes());
        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        let wav = parse_wav(&bytes).expect("valid WAV must parse");
        assert_eq!(wav.samples, vec![123]);
    }
}
