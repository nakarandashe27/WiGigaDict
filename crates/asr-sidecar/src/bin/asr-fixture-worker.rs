use serde::Serialize;
use std::env;
use std::fs::OpenOptions;
use std::process::ExitCode;
use wigigadict_protocol::AsrSegment;

#[derive(Serialize)]
struct FixtureRun {
    inference_ms: u64,
    text: String,
    segments: Vec<AsrSegment>,
}

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().map(String::as_str) != Some("run-whisper")
        || !has_pair(&arguments, "--mode", "cold")
        || !has_pair(&arguments, "--profile", "gpu")
        || !has_pair(&arguments, "--threads", "0")
        || value_after(&arguments, "--model").is_none()
        || value_after(&arguments, "--audio").is_none()
        || value_after(&arguments, "--sample").is_none()
    {
        return ExitCode::from(2);
    }
    let Some(output_path) = value_after(&arguments, "--output") else {
        return ExitCode::from(2);
    };
    let output = FixtureRun {
        inference_ms: 7,
        text: "fixture raw".into(),
        segments: vec![AsrSegment {
            start_ms: 0,
            end_ms: 500,
            text: "fixture raw".into(),
        }],
    };
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
        .and_then(|mut file| {
            serde_json::to_writer(&mut file, &output).map_err(std::io::Error::other)?;
            std::io::Write::write_all(&mut file, b"\n")?;
            std::io::Write::flush(&mut file)
        });
    if result.is_err() {
        return ExitCode::from(3);
    }
    ExitCode::SUCCESS
}

fn value_after<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn has_pair(arguments: &[String], name: &str, value: &str) -> bool {
    value_after(arguments, name) == Some(value)
}
