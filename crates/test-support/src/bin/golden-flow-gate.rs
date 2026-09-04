use serde::de::DeserializeOwned;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use wigigadict_test_support::golden_flow::{
    GoldenFlowRun, GoldenFlowThresholds, evaluate_golden_flow, validate_thresholds,
};

const MAX_THRESHOLDS_BYTES: u64 = 64 * 1024;
const MAX_EVIDENCE_BYTES: u64 = 4 * 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("golden-flow-gate: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [command, thresholds] if command == "check-thresholds" => {
            let thresholds: GoldenFlowThresholds =
                read_json(Path::new(thresholds), MAX_THRESHOLDS_BYTES)?;
            let violations = validate_thresholds(&thresholds);
            if !violations.is_empty() {
                return Err(violations.join("; "));
            }
            println!("golden-flow thresholds are valid");
            Ok(())
        }
        [command, thresholds, evidence, output] if command == "evaluate" => {
            let thresholds: GoldenFlowThresholds =
                read_json(Path::new(thresholds), MAX_THRESHOLDS_BYTES)?;
            let evidence: GoldenFlowRun = read_json(Path::new(evidence), MAX_EVIDENCE_BYTES)?;
            let report = evaluate_golden_flow(&thresholds, &evidence);
            write_report(Path::new(output), &report)?;
            println!(
                "golden-flow gate: {} ({} sessions, {} violations)",
                if report.passed { "passed" } else { "failed" },
                report.counts.sessions,
                report.violations.len()
            );
            if !report.passed {
                std::process::exit(2);
            }
            Ok(())
        }
        _ => Err(
            "usage: golden-flow-gate check-thresholds <thresholds.json> | evaluate <thresholds.json> <evidence.json> <new-report.json>"
                .into(),
        ),
    }
}

fn read_json<T: DeserializeOwned>(path: &Path, limit: u64) -> Result<T, String> {
    let metadata = fs::metadata(path).map_err(|_| "input file is unavailable".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err("input file size is outside the allowed boundary".into());
    }
    let reader = BufReader::new(File::open(path).map_err(|_| "input file cannot open".to_owned())?);
    serde_json::from_reader(reader)
        .map_err(|_| "input JSON is malformed or has unknown fields".into())
}

fn write_report(
    output: &Path,
    report: &wigigadict_test_support::golden_flow::GoldenFlowReport,
) -> Result<(), String> {
    if output.extension().and_then(|value| value.to_str()) != Some("json")
        || output.file_name().is_none()
        || output.exists()
    {
        return Err("output must be a new .json file".into());
    }
    let part = part_path(output)?;
    if part.exists() {
        return Err("staging report already exists".into());
    }
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&part)
        .map_err(|_| "staging report cannot be created".to_owned())?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, report)
        .map_err(|_| "report serialization failed".to_owned())?;
    writer
        .write_all(b"\n")
        .map_err(|_| "report write failed".to_owned())?;
    writer
        .flush()
        .map_err(|_| "report flush failed".to_owned())?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|_| "report sync failed".to_owned())?;
    drop(writer);
    fs::rename(&part, output).map_err(|_| "report promotion failed".to_owned())
}

fn part_path(output: &Path) -> Result<PathBuf, String> {
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "output filename is invalid".to_owned())?;
    Ok(output.with_file_name(format!("{name}.part")))
}
