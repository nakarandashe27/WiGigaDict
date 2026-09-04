use wigigadict_protocol::{
    AsrSegment, EngineKind, EngineOutput, LanguageHint, RuntimeSpec, ShellCommand,
    TranscribeRequest, WhisperProfile,
};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn request() -> TranscribeRequest {
    TranscribeRequest {
        request_id: "request-1".into(),
        attempt_id: "attempt-1".into(),
        lease_generation: 1,
        audio_path: r"C:\managed\audio\one.wav".into(),
        audio_sha256: HASH.into(),
        audio_byte_size: 32_000,
        audio_duration_ms: 1_000,
        language: LanguageHint::Auto,
        timeout_ms: 30_000,
        runtime: RuntimeSpec {
            engine: EngineKind::WhisperLargeV3TurboQ5,
            worker_path: r"C:\managed\installed\worker.exe".into(),
            worker_sha256: HASH.into(),
            model_path: r"C:\managed\installed\model.bin".into(),
            model_sha256: HASH.into(),
            profile: WhisperProfile::Vulkan,
        },
    }
}

#[test]
fn transcribe_command_roundtrips_with_an_exact_engine_and_profile() {
    let command = ShellCommand::Transcribe(Box::new(request()));
    command.validate().unwrap();
    let json = serde_json::to_string(&command).unwrap();
    assert!(json.contains(r#""type":"transcribe""#));
    assert!(json.contains("whisper_large_v3_turbo_q5"));
    assert!(json.contains("vulkan"));
    let decoded: ShellCommand = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, command);
}

#[test]
fn arbitrary_engine_or_profile_is_rejected_by_deserialization() {
    let valid = serde_json::to_value(ShellCommand::Transcribe(Box::new(request()))).unwrap();
    let mut engine = valid.clone();
    engine["runtime"]["engine"] = serde_json::json!("arbitrary_engine");
    assert!(serde_json::from_value::<ShellCommand>(engine).is_err());

    let mut profile = valid;
    profile["runtime"]["profile"] = serde_json::json!("cpu-t99");
    assert!(serde_json::from_value::<ShellCommand>(profile).is_err());
}

#[test]
fn relative_or_oversized_paths_are_rejected_before_engine_start() {
    let mut relative = request();
    relative.audio_path = "audio/one.wav".into();
    assert!(relative.validate().is_err());

    let mut oversized = request();
    oversized.runtime.worker_path = format!(r"C:\{}", "a".repeat(32_768));
    assert!(oversized.validate().is_err());
}

#[test]
fn segment_outside_wav_or_non_monotonic_timing_is_rejected() {
    let outside = EngineOutput {
        text: "text".into(),
        segments: vec![AsrSegment {
            start_ms: 0,
            end_ms: 1_001,
            text: "text".into(),
        }],
    };
    assert!(outside.validate(1_000).is_err());

    let non_monotonic = EngineOutput {
        text: "text".into(),
        segments: vec![
            AsrSegment {
                start_ms: 100,
                end_ms: 200,
                text: "a".into(),
            },
            AsrSegment {
                start_ms: 150,
                end_ms: 250,
                text: "b".into(),
            },
        ],
    };
    assert!(non_monotonic.validate(1_000).is_err());
}

#[test]
fn unknown_critical_transcribe_field_is_rejected() {
    let mut value = serde_json::to_value(ShellCommand::Transcribe(Box::new(request()))).unwrap();
    value["critical"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ShellCommand>(value).is_err());
}
