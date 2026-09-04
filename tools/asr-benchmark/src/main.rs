#![cfg_attr(
    not(any(
        feature = "whisper-cpu",
        feature = "whisper-vulkan",
        feature = "gigaam-cpu",
        feature = "gigaam-directml"
    )),
    allow(dead_code, unused_imports)
)]
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct Probe {
    schema_version: u32,
    kind: &'static str,
    bytes: u64,
    sha256: String,
    compatible: bool,
    reason_code: &'static str,
}

#[derive(Serialize)]
struct Segment {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

#[derive(Serialize)]
struct Run {
    schema_version: u32,
    run_id: String,
    engine: &'static str,
    adapter: &'static str,
    runtime: &'static str,
    profile: String,
    mode: String,
    sample_id: String,
    language: String,
    model_sha256: String,
    model_bytes: u64,
    audio_sha256: String,
    audio_duration_ms: u64,
    load_ms: u64,
    inference_ms: u64,
    total_ms: u64,
    rtf: f64,
    peak_working_set_bytes: Option<u64>,
    peak_vram_bytes: Option<u64>,
    average_incremental_watts: Option<f64>,
    energy_kwh: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n_threads: Option<i32>,
    text: String,
    segments: Vec<Segment>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("asr-benchmark: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("probe-whisper") => {
            let path = Path::new(required(&args, "--model")?);
            let mut file = File::open(path)?;
            let mut magic = [0_u8; 4];
            file.read_exact(&mut magic)?;
            let compatible = magic == *b"lmgg";
            print_json(&Probe {
                schema_version: 1,
                kind: "whisper_ggml",
                bytes: file.metadata()?.len(),
                sha256: sha256(path)?,
                compatible,
                reason_code: if compatible {
                    "compatible_ggml"
                } else {
                    "incompatible_magic"
                },
            })
        }
        Some("probe-gigaam") => {
            let dir = Path::new(required(&args, "--model-dir")?);
            let model = match optional(&args, "--quantization", "fp32") {
                "fp32" => dir.join("model.onnx"),
                "int8" => dir.join("model.int8.onnx"),
                value => return Err(format!("unsupported quantization: {value}").into()),
            };
            let compatible = model.is_file() && dir.join("vocab.txt").is_file();
            print_json(&Probe {
                schema_version: 1,
                kind: "gigaam_ctc_onnx",
                bytes: model.metadata().map(|m| m.len()).unwrap_or(0),
                sha256: if model.is_file() {
                    sha256(&model)?
                } else {
                    String::new()
                },
                compatible,
                reason_code: if compatible {
                    "compatible_ctc_onnx_vocab"
                } else {
                    "missing_ctc_onnx_or_vocab"
                },
            })
        }
        Some("run-whisper") => run_whisper(&args),
        Some("run-gigaam") => run_gigaam(&args),
        Some("fault-after-load") => fault_after_load(&args),
        _ => {
            Err("usage: probe-whisper|probe-gigaam|run-whisper|run-gigaam|fault-after-load --engine whisper|gigaam".into())
        }
    }
}

fn required<'a>(args: &'a [String], name: &str) -> Result<&'a str, Box<dyn Error>> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
        .ok_or_else(|| format!("missing {name}").into())
}

fn optional<'a>(args: &'a [String], name: &str, fallback: &'a str) -> &'a str {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
        .unwrap_or(fallback)
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn run_mode(args: &[String]) -> Result<&str, Box<dyn Error>> {
    match required(args, "--mode")? {
        value @ ("cold" | "warm") => Ok(value),
        value => Err(format!("unsupported --mode: {value}; expected cold or warm").into()),
    }
}

fn whisper_profile(args: &[String], n_threads: i32) -> Result<&str, Box<dyn Error>> {
    let profile = required(args, "--profile")?;
    match profile {
        "gpu" => Ok(profile),
        "cpu" if n_threads == 0 => Ok(profile),
        "cpu" => {
            Err("profile cpu requires --threads 0; use cpu-tN for a pinned thread count".into())
        }
        value if value.starts_with("cpu-t") => {
            let declared = value
                .strip_prefix("cpu-t")
                .and_then(|raw| raw.parse::<i32>().ok())
                .filter(|threads| (1..=64).contains(threads))
                .ok_or_else(|| format!("unsupported --profile: {value}"))?;
            if declared != n_threads {
                return Err(format!(
                    "profile {value} requires --threads {declared}, got {n_threads}"
                )
                .into());
            }
            Ok(profile)
        }
        value => Err(format!(
            "unsupported --profile: {value}; expected gpu, cpu, or cpu-t1..cpu-t64"
        )
        .into()),
    }
}

fn accelerator_profile(args: &[String]) -> Result<&str, Box<dyn Error>> {
    match required(args, "--profile")? {
        value @ ("cpu" | "gpu") => Ok(value),
        value => Err(format!("unsupported --profile: {value}; expected cpu or gpu").into()),
    }
}

fn bounded_i32(
    args: &[String],
    name: &str,
    fallback: i32,
    minimum: i32,
    maximum: i32,
) -> Result<i32, Box<dyn Error>> {
    let value = match args.iter().position(|arg| arg == name) {
        Some(index) => {
            let raw = args
                .get(index + 1)
                .ok_or_else(|| format!("missing value for {name}"))?;
            raw.parse::<i32>()
                .map_err(|_| format!("invalid {name}: {raw}"))?
        }
        None => fallback,
    };
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be in {minimum}..={maximum}, got {value}").into());
    }
    Ok(value)
}

fn fault_after_load(args: &[String]) -> Result<(), Box<dyn Error>> {
    match required(args, "--engine")? {
        "whisper" => {
            let n_threads = bounded_i32(args, "--threads", 0, 0, 64)?;
            let profile = whisper_profile(args, n_threads)?;
            #[cfg(any(feature = "whisper-cpu", feature = "whisper-vulkan"))]
            {
                use transcribe_rs::whisper_cpp::{WhisperEngine, WhisperLoadParams};

                if profile == "gpu" && !cfg!(feature = "whisper-vulkan") {
                    return Err("gpu profile requires whisper-vulkan feature".into());
                }
                let model = PathBuf::from(required(args, "--model")?);
                let _engine = WhisperEngine::load_with_params(
                    &model,
                    WhisperLoadParams {
                        use_gpu: profile == "gpu",
                        flash_attn: true,
                        gpu_device: -1,
                    },
                )?;
                eprintln!(
                    "intentional worker fault after verified model load: engine=whisper profile={profile}"
                );
                std::process::exit(86);
            }
            #[cfg(not(any(feature = "whisper-cpu", feature = "whisper-vulkan")))]
            {
                let _ = profile;
                Err("compile with --features whisper-cpu or whisper-vulkan".into())
            }
        }
        "gigaam" => {
            let profile = accelerator_profile(args)?;
            #[cfg(any(feature = "gigaam-cpu", feature = "gigaam-directml"))]
            {
                use transcribe_rs::onnx::Quantization;
                use transcribe_rs::onnx::gigaam::GigaAMModel;
                use transcribe_rs::{OrtAccelerator, set_ort_accelerator};

                if profile == "gpu" && !cfg!(feature = "gigaam-directml") {
                    return Err("gpu profile requires gigaam-directml feature".into());
                }
                set_ort_accelerator(if profile == "gpu" {
                    OrtAccelerator::DirectMl
                } else {
                    OrtAccelerator::CpuOnly
                });
                let dir = PathBuf::from(required(args, "--model-dir")?);
                let quantization = match optional(args, "--quantization", "fp32") {
                    "fp32" => Quantization::FP32,
                    "int8" => Quantization::Int8,
                    value => return Err(format!("unsupported quantization: {value}").into()),
                };
                let _engine = GigaAMModel::load(&dir, &quantization)?;
                eprintln!(
                    "intentional worker fault after verified model load: engine=gigaam profile={profile}"
                );
                std::process::exit(86);
            }
            #[cfg(not(any(feature = "gigaam-cpu", feature = "gigaam-directml")))]
            {
                let _ = profile;
                Err("compile with --features gigaam-cpu or gigaam-directml".into())
            }
        }
        value => Err(format!("unsupported engine: {value}").into()),
    }
}

fn sha256(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn print_json(value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn write_record(args: &[String], value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let path = Path::new(required(args, "--output")?);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true);
    if flag(args, "--append") {
        let expected = required(args, "--expected-existing-records")?
            .parse::<usize>()
            .map_err(|_| "--expected-existing-records must be a non-negative integer")?;
        let existing = fs::read_to_string(path)?;
        let actual = existing
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        if actual != expected {
            return Err(format!(
                "refusing evidence append: expected {expected} existing records, found {actual}"
            )
            .into());
        }
        options.append(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path).map_err(|error| {
        if path.exists() && !flag(args, "--append") {
            format!(
                "output already exists: {}; use a new path or explicit --append --expected-existing-records N for one predeclared matrix ({error})",
                path.display()
            )
        } else {
            format!("unable to open output {}: {error}", path.display())
        }
    })?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn wav_duration_ms(path: &Path) -> Result<u64, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    wav_duration_ms_from_bytes(&bytes)
}

fn wav_duration_ms_from_bytes(bytes: &[u8]) -> Result<u64, Box<dyn Error>> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("invalid WAV".into());
    }
    let riff_size = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let riff_end = 8_usize
        .checked_add(riff_size)
        .ok_or("WAV RIFF size overflow")?;
    if riff_end != bytes.len() {
        return Err(format!(
            "WAV RIFF size mismatch: header declares {riff_end} bytes, file has {}",
            bytes.len()
        )
        .into());
    }

    let mut offset = 12_usize;
    let mut format_seen = false;
    let mut data_bytes = None;
    while offset < riff_end {
        let header_end = offset.checked_add(8).ok_or("WAV chunk header overflow")?;
        if header_end > riff_end {
            return Err("truncated WAV chunk header".into());
        }
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes(bytes[offset + 4..header_end].try_into()?) as usize;
        let payload_start = header_end;
        let payload_end = payload_start
            .checked_add(chunk_size)
            .ok_or("WAV chunk size overflow")?;
        if payload_end > riff_end {
            return Err("WAV chunk extends past RIFF boundary".into());
        }

        match chunk_id {
            b"fmt " => {
                if format_seen {
                    return Err("duplicate WAV fmt chunk".into());
                }
                if chunk_size < 16 {
                    return Err("WAV fmt chunk is too short".into());
                }
                let format_code =
                    u16::from_le_bytes(bytes[payload_start..payload_start + 2].try_into()?);
                let channels =
                    u16::from_le_bytes(bytes[payload_start + 2..payload_start + 4].try_into()?);
                let rate =
                    u32::from_le_bytes(bytes[payload_start + 4..payload_start + 8].try_into()?);
                let byte_rate =
                    u32::from_le_bytes(bytes[payload_start + 8..payload_start + 12].try_into()?);
                let block_align =
                    u16::from_le_bytes(bytes[payload_start + 12..payload_start + 14].try_into()?);
                let bits =
                    u16::from_le_bytes(bytes[payload_start + 14..payload_start + 16].try_into()?);
                if format_code != 1
                    || channels != 1
                    || rate != 16_000
                    || byte_rate != 32_000
                    || block_align != 2
                    || bits != 16
                {
                    return Err(
                        "expected integer PCM 16000 Hz/mono/16-bit with valid byte rate and block alignment"
                            .into(),
                    );
                }
                format_seen = true;
            }
            b"data" => {
                if data_bytes.is_some() {
                    return Err("duplicate WAV data chunk".into());
                }
                data_bytes = Some(chunk_size as u64);
            }
            _ => {}
        }

        offset = payload_end
            .checked_add(chunk_size & 1)
            .ok_or("WAV chunk padding overflow")?;
        if offset > riff_end {
            return Err("missing WAV chunk padding byte".into());
        }
    }
    if !format_seen {
        return Err("missing WAV fmt chunk".into());
    }
    let data = data_bytes.ok_or("missing WAV data chunk")?;
    if data % 2 != 0 {
        return Err("WAV data length is not block-aligned".into());
    }
    Ok(data * 1_000 / 32_000)
}

fn validate_segments(segments: &[Segment], duration_ms: u64) -> Result<(), Box<dyn Error>> {
    let mut previous_end = 0;
    for (index, segment) in segments.iter().enumerate() {
        if segment.start_ms < previous_end || segment.end_ms < segment.start_ms {
            return Err(format!("ASR segment {index} is non-monotonic").into());
        }
        if segment.end_ms > duration_ms {
            return Err(format!(
                "ASR segment {index} ends at {} ms beyond WAV duration {duration_ms} ms",
                segment.end_ms
            )
            .into());
        }
        previous_end = segment.end_ms;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn base_run(
    engine: &'static str,
    runtime: &'static str,
    args: &[String],
    model: &Path,
    audio: &Path,
    load_ms: u64,
    inference_ms: u64,
    text: String,
    segments: Vec<Segment>,
    profile: &str,
    mode: &str,
) -> Result<Run, Box<dyn Error>> {
    let duration = wav_duration_ms(audio)?;
    validate_segments(&segments, duration)?;
    let sample_id = required(args, "--sample")?.to_string();
    Ok(Run {
        schema_version: 1,
        run_id: format!(
            "{}-{}-{}-{}",
            engine,
            profile,
            sample_id,
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
        ),
        engine,
        adapter: "transcribe-rs=0.3.11",
        runtime,
        profile: profile.to_string(),
        mode: mode.to_string(),
        sample_id,
        language: required(args, "--language")?.to_string(),
        model_sha256: sha256(model)?,
        model_bytes: model.metadata()?.len(),
        audio_sha256: sha256(audio)?,
        audio_duration_ms: duration,
        load_ms,
        inference_ms,
        total_ms: load_ms + inference_ms,
        rtf: inference_ms as f64 / duration.max(1) as f64,
        peak_working_set_bytes: peak_working_set(),
        peak_vram_bytes: None,
        average_incremental_watts: None,
        energy_kwh: None,
        n_threads: None,
        text,
        segments,
    })
}

#[cfg(any(feature = "whisper-cpu", feature = "whisper-vulkan"))]
fn run_whisper(args: &[String]) -> Result<(), Box<dyn Error>> {
    use transcribe_rs::audio::read_wav_samples;
    use transcribe_rs::whisper_cpp::{WhisperEngine, WhisperInferenceParams, WhisperLoadParams};

    let model = PathBuf::from(required(args, "--model")?);
    let audio = PathBuf::from(required(args, "--audio")?);
    let n_threads = bounded_i32(args, "--threads", 0, 0, 64)?;
    let profile = whisper_profile(args, n_threads)?;
    let mode = run_mode(args)?;
    if profile == "gpu" && !cfg!(feature = "whisper-vulkan") {
        return Err("gpu profile requires whisper-vulkan feature".into());
    }
    let samples = read_wav_samples(&audio)?;
    let started = Instant::now();
    let mut engine = WhisperEngine::load_with_params(
        &model,
        WhisperLoadParams {
            use_gpu: profile == "gpu",
            flash_attn: true,
            gpu_device: -1,
        },
    )?;
    let load_ms = started.elapsed().as_millis() as u64;
    if mode == "warm" {
        engine.transcribe_with(
            &samples,
            &WhisperInferenceParams {
                language: Some(required(args, "--language")?.to_string()),
                n_threads,
                ..Default::default()
            },
        )?;
    }
    let started = Instant::now();
    let result = engine.transcribe_with(
        &samples,
        &WhisperInferenceParams {
            language: Some(required(args, "--language")?.to_string()),
            n_threads,
            ..Default::default()
        },
    )?;
    let inference_ms = started.elapsed().as_millis() as u64;
    let segments = result
        .segments
        .unwrap_or_default()
        .into_iter()
        .map(|segment| Segment {
            start_ms: (segment.start.max(0.0) * 1_000.0).round() as u64,
            end_ms: (segment.end.max(0.0) * 1_000.0).round() as u64,
            text: segment.text,
        })
        .collect();
    let mut run = base_run(
        "whisper",
        if profile == "gpu" {
            "whisper.cpp-vulkan"
        } else {
            "whisper.cpp-cpu"
        },
        args,
        &model,
        &audio,
        load_ms,
        inference_ms,
        result.text,
        segments,
        profile,
        mode,
    )?;
    run.n_threads = Some(n_threads);
    write_record(args, &run)
}

#[cfg(not(any(feature = "whisper-cpu", feature = "whisper-vulkan")))]
fn run_whisper(_args: &[String]) -> Result<(), Box<dyn Error>> {
    Err("compile with --features whisper-cpu or whisper-vulkan".into())
}

#[cfg(any(feature = "gigaam-cpu", feature = "gigaam-directml"))]
fn run_gigaam(args: &[String]) -> Result<(), Box<dyn Error>> {
    use transcribe_rs::audio::read_wav_samples;
    use transcribe_rs::onnx::Quantization;
    use transcribe_rs::onnx::gigaam::{GigaAMModel, GigaAMParams};
    use transcribe_rs::{OrtAccelerator, set_ort_accelerator};

    let dir = PathBuf::from(required(args, "--model-dir")?);
    let (quantization, model) = match optional(args, "--quantization", "fp32") {
        "fp32" => (Quantization::FP32, dir.join("model.onnx")),
        "int8" => (Quantization::Int8, dir.join("model.int8.onnx")),
        value => return Err(format!("unsupported quantization: {value}").into()),
    };
    let audio = PathBuf::from(required(args, "--audio")?);
    let profile = accelerator_profile(args)?;
    let mode = run_mode(args)?;
    if profile == "gpu" && !cfg!(feature = "gigaam-directml") {
        return Err("gpu profile requires gigaam-directml feature".into());
    }
    set_ort_accelerator(if profile == "gpu" {
        OrtAccelerator::DirectMl
    } else {
        OrtAccelerator::CpuOnly
    });
    let samples = read_wav_samples(&audio)?;
    let started = Instant::now();
    let mut engine = GigaAMModel::load(&dir, &quantization)?;
    let load_ms = started.elapsed().as_millis() as u64;
    if mode == "warm" {
        engine.transcribe_with(
            &samples,
            &GigaAMParams {
                language: Some(required(args, "--language")?.to_string()),
            },
        )?;
    }
    let started = Instant::now();
    let result = engine.transcribe_with(
        &samples,
        &GigaAMParams {
            language: Some(required(args, "--language")?.to_string()),
        },
    )?;
    let inference_ms = started.elapsed().as_millis() as u64;
    let segments = result
        .segments
        .unwrap_or_default()
        .into_iter()
        .map(|segment| Segment {
            start_ms: (segment.start.max(0.0) * 1_000.0).round() as u64,
            end_ms: (segment.end.max(0.0) * 1_000.0).round() as u64,
            text: segment.text,
        })
        .collect();
    let run = base_run(
        "gigaam",
        if profile == "gpu" {
            "onnxruntime-directml"
        } else {
            "onnxruntime-cpu"
        },
        args,
        &model,
        &audio,
        load_ms,
        inference_ms,
        result.text,
        segments,
        profile,
        mode,
    )?;
    write_record(args, &run)
}

#[cfg(not(any(feature = "gigaam-cpu", feature = "gigaam-directml")))]
fn run_gigaam(_args: &[String]) -> Result<(), Box<dyn Error>> {
    Err("compile with --features gigaam-cpu or gigaam-directml".into())
}

#[cfg(windows)]
fn peak_working_set() -> Option<u64> {
    use std::ffi::c_void;
    #[repr(C)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(process: *mut c_void, counters: *mut Counters, size: u32) -> i32;
    }
    let mut value = Counters {
        cb: size_of::<Counters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let process = unsafe { GetCurrentProcess() };
    let ok = unsafe { GetProcessMemoryInfo(process, &mut value, value.cb) };
    (ok != 0).then_some(value.peak_working_set_size as u64)
}

#[cfg(not(windows))]
fn peak_working_set() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_i32_accepts_fallback_and_explicit_value() {
        let args = vec!["run-whisper".to_string()];
        assert_eq!(bounded_i32(&args, "--threads", 0, 0, 64).unwrap(), 0);

        let args = vec![
            "run-whisper".to_string(),
            "--threads".to_string(),
            "16".to_string(),
        ];
        assert_eq!(bounded_i32(&args, "--threads", 0, 0, 64).unwrap(), 16);
    }

    #[test]
    fn bounded_i32_rejects_missing_invalid_or_out_of_range_value() {
        for args in [
            vec!["run-whisper", "--threads"],
            vec!["run-whisper", "--threads", "many"],
            vec!["run-whisper", "--threads", "-1"],
            vec!["run-whisper", "--threads", "65"],
        ] {
            let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert!(bounded_i32(&args, "--threads", 0, 0, 64).is_err());
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn wav_chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(id);
        chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        chunk.extend_from_slice(payload);
        if !payload.len().is_multiple_of(2) {
            chunk.push(0);
        }
        chunk
    }

    fn pcm_wav(extra_chunks: &[Vec<u8>], data_bytes: usize) -> Vec<u8> {
        let mut payload = b"WAVE".to_vec();
        for chunk in extra_chunks {
            payload.extend_from_slice(chunk);
        }
        let mut format = Vec::new();
        format.extend_from_slice(&1_u16.to_le_bytes());
        format.extend_from_slice(&1_u16.to_le_bytes());
        format.extend_from_slice(&16_000_u32.to_le_bytes());
        format.extend_from_slice(&32_000_u32.to_le_bytes());
        format.extend_from_slice(&2_u16.to_le_bytes());
        format.extend_from_slice(&16_u16.to_le_bytes());
        payload.extend_from_slice(&wav_chunk(b"fmt ", &format));
        payload.extend_from_slice(&wav_chunk(b"data", &vec![0_u8; data_bytes]));

        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        wav.extend_from_slice(&payload);
        wav
    }

    #[test]
    fn profile_and_mode_validation_reject_relabeling() {
        assert_eq!(
            run_mode(&args(&["run-whisper", "--mode", "cold"])).unwrap(),
            "cold"
        );
        assert!(run_mode(&args(&["run-whisper", "--mode", "tepid"])).is_err());
        assert!(run_mode(&args(&["run-whisper"])).is_err());

        assert_eq!(
            whisper_profile(&args(&["run-whisper", "--profile", "cpu-t16"]), 16).unwrap(),
            "cpu-t16"
        );
        assert!(whisper_profile(&args(&["run-whisper", "--profile", "gpu-human"]), 0).is_err());
        assert!(whisper_profile(&args(&["run-whisper", "--profile", "cpu-t16"]), 8).is_err());
        assert!(accelerator_profile(&args(&["run-gigaam", "--profile", "cpu-t16"])).is_err());
    }

    #[test]
    fn wav_parser_traverses_unknown_and_odd_sized_chunks() {
        let junk = wav_chunk(b"JUNK", &[1, 2, 3]);
        let wav = pcm_wav(&[junk], 32_000);
        assert_eq!(wav_duration_ms_from_bytes(&wav).unwrap(), 1_000);
    }

    #[test]
    fn wav_parser_rejects_truncation_and_invalid_format_fields() {
        let mut truncated = pcm_wav(&[], 32_000);
        truncated.pop();
        assert!(wav_duration_ms_from_bytes(&truncated).is_err());

        let mut bad_byte_rate = pcm_wav(&[], 32_000);
        let byte_rate_offset = 12 + 8 + 8;
        bad_byte_rate[byte_rate_offset..byte_rate_offset + 4]
            .copy_from_slice(&31_999_u32.to_le_bytes());
        assert!(wav_duration_ms_from_bytes(&bad_byte_rate).is_err());
    }

    #[test]
    fn segment_validation_rejects_end_past_wav_duration() {
        let segments = vec![Segment {
            start_ms: 900,
            end_ms: 1_001,
            text: "tail".to_string(),
        }];
        assert!(validate_segments(&segments, 1_000).is_err());
    }

    #[test]
    fn output_requires_new_path_or_explicit_append() {
        let path = std::env::temp_dir().join(format!(
            "wigigadict-asr-benchmark-{}.ndjson",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path_text = path.to_string_lossy().into_owned();
        let first = args(&["run-whisper", "--output", &path_text]);
        write_record(&first, &serde_json::json!({"run": 1})).unwrap();
        assert!(write_record(&first, &serde_json::json!({"run": 2})).is_err());

        let ambiguous_append = args(&["run-whisper", "--output", &path_text, "--append"]);
        assert!(write_record(&ambiguous_append, &serde_json::json!({"run": 2})).is_err());

        let wrong_count = args(&[
            "run-whisper",
            "--output",
            &path_text,
            "--append",
            "--expected-existing-records",
            "2",
        ]);
        assert!(write_record(&wrong_count, &serde_json::json!({"run": 2})).is_err());

        let append_args = args(&[
            "run-whisper",
            "--output",
            &path_text,
            "--append",
            "--expected-existing-records",
            "1",
        ]);
        write_record(&append_args, &serde_json::json!({"run": 2})).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
        fs::remove_file(path).unwrap();
    }
}
