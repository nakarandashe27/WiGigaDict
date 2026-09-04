use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const GOLDEN_FLOW_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenFlowThresholds {
    pub schema_version: u32,
    pub gate_id: String,
    pub required_os_build: u32,
    pub required_runtime_profile: String,
    pub required_sessions: usize,
    pub required_codex_sessions: usize,
    pub required_vscode_sessions: usize,
    pub minimum_short_sessions: usize,
    pub minimum_medium_sessions: usize,
    pub minimum_long_sessions: usize,
    pub maximum_irrecoverable_results: usize,
    pub maximum_intent_changed_sessions: usize,
    pub maximum_corrupt_audio_sessions: usize,
    pub minimum_quick_review_sessions: usize,
    pub maximum_release_to_terminal_p50_ms: u64,
    pub maximum_release_to_terminal_p95_ms: u64,
    pub maximum_inference_p95_ms: u64,
    pub maximum_rtf_p95: f64,
    pub maximum_peak_ram_bytes: u64,
    pub maximum_peak_vram_bytes: u64,
    pub required_gates: RequiredGates,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredGates {
    pub offline_deny_all: bool,
    pub crash_restart: bool,
    pub load_admission: bool,
    pub cleanup_corpus: bool,
    pub marker_redaction: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenFlowRun {
    pub schema_version: u32,
    pub gate_id: String,
    pub os_build: u32,
    pub runtime_profile: String,
    pub gates: GateEvidence,
    pub sessions: Vec<GoldenSession>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateEvidence {
    pub offline_deny_all: bool,
    pub crash_restart: bool,
    pub load_admission: bool,
    pub cleanup_corpus: bool,
    pub marker_redaction: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenSession {
    pub ordinal: usize,
    pub session_id: String,
    pub target: TargetFamily,
    pub duration_class: DurationClass,
    pub terminal_outcome: TerminalOutcome,
    pub evidence_class: DeliveryEvidence,
    pub audio_recoverable: bool,
    pub text_recoverable: bool,
    pub quick_review: bool,
    pub intent_changed: bool,
    pub corrupt_audio: bool,
    pub release_to_terminal_ms: u64,
    pub inference_ms: u64,
    pub audio_duration_ms: u64,
    pub peak_ram_bytes: u64,
    pub peak_vram_bytes: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetFamily {
    Codex,
    Vscode,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DurationClass {
    Short,
    Medium,
    Long,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcome {
    Delivered,
    Uncertain,
    Failed,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryEvidence {
    TargetAck,
    CertifiedTransport,
    TransportOnly,
    None,
}

impl DeliveryEvidence {
    fn confirms_delivery(self) -> bool {
        matches!(self, Self::TargetAck | Self::CertifiedTransport)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GoldenFlowReport {
    pub schema_version: u32,
    pub gate_id: String,
    pub passed: bool,
    pub counts: GateCounts,
    pub performance: GatePerformance,
    pub violations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GateCounts {
    pub sessions: usize,
    pub codex: usize,
    pub vscode: usize,
    pub short: usize,
    pub medium: usize,
    pub long: usize,
    pub delivered: usize,
    pub uncertain: usize,
    pub failed: usize,
    pub irrecoverable: usize,
    pub quick_review: usize,
    pub intent_changed: usize,
    pub corrupt_audio: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GatePerformance {
    pub release_to_terminal_p50_ms: u64,
    pub release_to_terminal_p95_ms: u64,
    pub inference_p95_ms: u64,
    pub rtf_p95: f64,
    pub peak_ram_bytes: u64,
    pub peak_vram_bytes: u64,
}

pub fn validate_thresholds(thresholds: &GoldenFlowThresholds) -> Vec<String> {
    let mut errors = Vec::new();
    if thresholds.schema_version != GOLDEN_FLOW_SCHEMA_VERSION {
        errors.push("unsupported threshold schema_version".into());
    }
    if !machine_token(&thresholds.gate_id) {
        errors.push("gate_id must be a bounded machine token".into());
    }
    if !machine_token(&thresholds.required_runtime_profile) {
        errors.push("required_runtime_profile must be a bounded machine token".into());
    }
    if thresholds.required_os_build != 19_045 {
        errors.push("Step 16 thresholds are frozen for Windows 10 build 19045".into());
    }
    if thresholds.required_sessions != 100
        || thresholds.required_codex_sessions + thresholds.required_vscode_sessions
            != thresholds.required_sessions
    {
        errors.push("target counts must partition exactly 100 sessions".into());
    }
    if thresholds.minimum_short_sessions
        + thresholds.minimum_medium_sessions
        + thresholds.minimum_long_sessions
        > thresholds.required_sessions
    {
        errors.push("minimum duration counts exceed required sessions".into());
    }
    if thresholds.maximum_irrecoverable_results != 0
        || thresholds.maximum_intent_changed_sessions != 0
        || thresholds.maximum_corrupt_audio_sessions != 0
    {
        errors.push("loss, intent change, and corrupt audio ceilings must remain zero".into());
    }
    if thresholds.minimum_quick_review_sessions <= thresholds.required_sessions / 2
        || thresholds.minimum_quick_review_sessions > thresholds.required_sessions
    {
        errors.push("quick-review threshold must be a strict majority".into());
    }
    if thresholds.maximum_release_to_terminal_p50_ms > thresholds.maximum_release_to_terminal_p95_ms
        || thresholds.maximum_inference_p95_ms == 0
        || !thresholds.maximum_rtf_p95.is_finite()
        || thresholds.maximum_rtf_p95 <= 0.0
        || thresholds.maximum_peak_ram_bytes == 0
        || thresholds.maximum_peak_vram_bytes == 0
    {
        errors.push("performance thresholds are invalid".into());
    }
    if !thresholds.required_gates.offline_deny_all
        || !thresholds.required_gates.crash_restart
        || !thresholds.required_gates.load_admission
        || !thresholds.required_gates.cleanup_corpus
        || !thresholds.required_gates.marker_redaction
    {
        errors.push("all blocker regression gates must remain required".into());
    }
    errors
}

#[must_use]
pub fn evaluate_golden_flow(
    thresholds: &GoldenFlowThresholds,
    run: &GoldenFlowRun,
) -> GoldenFlowReport {
    let mut violations = validate_thresholds(thresholds);
    if run.schema_version != GOLDEN_FLOW_SCHEMA_VERSION {
        violations.push("unsupported run schema_version".into());
    }
    if run.gate_id != thresholds.gate_id {
        violations.push("run gate_id does not match frozen thresholds".into());
    }
    if run.os_build != thresholds.required_os_build {
        violations.push("run OS build does not match the frozen Windows baseline".into());
    }
    if run.runtime_profile != thresholds.required_runtime_profile {
        violations.push("run runtime profile does not match the frozen profile".into());
    }
    require_gate(
        thresholds.required_gates.offline_deny_all,
        run.gates.offline_deny_all,
        "offline_deny_all",
        &mut violations,
    );
    require_gate(
        thresholds.required_gates.crash_restart,
        run.gates.crash_restart,
        "crash_restart",
        &mut violations,
    );
    require_gate(
        thresholds.required_gates.load_admission,
        run.gates.load_admission,
        "load_admission",
        &mut violations,
    );
    require_gate(
        thresholds.required_gates.cleanup_corpus,
        run.gates.cleanup_corpus,
        "cleanup_corpus",
        &mut violations,
    );
    require_gate(
        thresholds.required_gates.marker_redaction,
        run.gates.marker_redaction,
        "marker_redaction",
        &mut violations,
    );

    let mut ordinals = BTreeSet::new();
    let mut session_ids = BTreeSet::new();
    let mut counts = GateCounts {
        sessions: run.sessions.len(),
        codex: 0,
        vscode: 0,
        short: 0,
        medium: 0,
        long: 0,
        delivered: 0,
        uncertain: 0,
        failed: 0,
        irrecoverable: 0,
        quick_review: 0,
        intent_changed: 0,
        corrupt_audio: 0,
    };
    let mut release = Vec::with_capacity(run.sessions.len());
    let mut inference = Vec::with_capacity(run.sessions.len());
    let mut rtf = Vec::with_capacity(run.sessions.len());
    let mut peak_ram = 0;
    let mut peak_vram = 0;

    for session in &run.sessions {
        if !ordinals.insert(session.ordinal)
            || !(1..=thresholds.required_sessions).contains(&session.ordinal)
        {
            violations.push(format!(
                "session ordinal {} is duplicate or out of range",
                session.ordinal
            ));
        }
        if !machine_token(&session.session_id) || !session_ids.insert(session.session_id.clone()) {
            violations.push(format!(
                "session {} has an invalid or duplicate id",
                session.ordinal
            ));
        }
        match session.target {
            TargetFamily::Codex => counts.codex += 1,
            TargetFamily::Vscode => counts.vscode += 1,
        }
        match session.duration_class {
            DurationClass::Short => counts.short += 1,
            DurationClass::Medium => counts.medium += 1,
            DurationClass::Long => counts.long += 1,
        }
        match session.terminal_outcome {
            TerminalOutcome::Delivered => {
                counts.delivered += 1;
                if !session.evidence_class.confirms_delivery() {
                    violations.push(format!(
                        "session {} claims delivered without strong evidence",
                        session.ordinal
                    ));
                }
            }
            TerminalOutcome::Uncertain => counts.uncertain += 1,
            TerminalOutcome::Failed => counts.failed += 1,
        }
        if !session.audio_recoverable && !session.text_recoverable {
            counts.irrecoverable += 1;
        }
        counts.quick_review += usize::from(session.quick_review);
        counts.intent_changed += usize::from(session.intent_changed);
        counts.corrupt_audio += usize::from(session.corrupt_audio);
        release.push(session.release_to_terminal_ms);
        inference.push(session.inference_ms);
        if session.audio_duration_ms == 0 {
            violations.push(format!(
                "session {} has zero audio duration",
                session.ordinal
            ));
        } else {
            rtf.push(session.inference_ms as f64 / session.audio_duration_ms as f64);
        }
        peak_ram = peak_ram.max(session.peak_ram_bytes);
        peak_vram = peak_vram.max(session.peak_vram_bytes);
    }

    let performance = GatePerformance {
        release_to_terminal_p50_ms: percentile_u64(&mut release, 50),
        release_to_terminal_p95_ms: percentile_u64(&mut release, 95),
        inference_p95_ms: percentile_u64(&mut inference, 95),
        rtf_p95: percentile_f64(&mut rtf, 95),
        peak_ram_bytes: peak_ram,
        peak_vram_bytes: peak_vram,
    };
    compare_counts(thresholds, &counts, &mut violations);
    compare_performance(thresholds, &performance, &mut violations);

    GoldenFlowReport {
        schema_version: GOLDEN_FLOW_SCHEMA_VERSION,
        gate_id: thresholds.gate_id.clone(),
        passed: violations.is_empty(),
        counts,
        performance,
        violations,
    }
}

fn require_gate(required: bool, actual: bool, name: &str, violations: &mut Vec<String>) {
    if required && !actual {
        violations.push(format!("required gate {name} did not pass"));
    }
}

fn compare_counts(
    thresholds: &GoldenFlowThresholds,
    counts: &GateCounts,
    violations: &mut Vec<String>,
) {
    if counts.sessions != thresholds.required_sessions
        || counts.codex != thresholds.required_codex_sessions
        || counts.vscode != thresholds.required_vscode_sessions
    {
        violations.push("session or target matrix count does not match thresholds".into());
    }
    if counts.short < thresholds.minimum_short_sessions
        || counts.medium < thresholds.minimum_medium_sessions
        || counts.long < thresholds.minimum_long_sessions
    {
        violations.push("duration-class coverage is below thresholds".into());
    }
    if counts.irrecoverable > thresholds.maximum_irrecoverable_results
        || counts.intent_changed > thresholds.maximum_intent_changed_sessions
        || counts.corrupt_audio > thresholds.maximum_corrupt_audio_sessions
        || counts.quick_review < thresholds.minimum_quick_review_sessions
    {
        violations.push("reliability or quality counts do not pass thresholds".into());
    }
}

fn compare_performance(
    thresholds: &GoldenFlowThresholds,
    performance: &GatePerformance,
    violations: &mut Vec<String>,
) {
    if performance.release_to_terminal_p50_ms > thresholds.maximum_release_to_terminal_p50_ms
        || performance.release_to_terminal_p95_ms > thresholds.maximum_release_to_terminal_p95_ms
        || performance.inference_p95_ms > thresholds.maximum_inference_p95_ms
        || performance.rtf_p95 > thresholds.maximum_rtf_p95
        || performance.peak_ram_bytes > thresholds.maximum_peak_ram_bytes
        || performance.peak_vram_bytes > thresholds.maximum_peak_vram_bytes
    {
        violations.push("performance or resource metrics exceed frozen thresholds".into());
    }
}

fn percentile_u64(values: &mut [u64], percentile: usize) -> u64 {
    values.sort_unstable();
    values
        .get(nearest_rank(values.len(), percentile))
        .copied()
        .unwrap_or_default()
}

fn percentile_f64(values: &mut [f64], percentile: usize) -> f64 {
    values.sort_by(f64::total_cmp);
    values
        .get(nearest_rank(values.len(), percentile))
        .copied()
        .unwrap_or_default()
}

fn nearest_rank(length: usize, percentile: usize) -> usize {
    if length == 0 {
        return 0;
    }
    (length * percentile).div_ceil(100).saturating_sub(1)
}

fn machine_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thresholds() -> GoldenFlowThresholds {
        GoldenFlowThresholds {
            schema_version: 1,
            gate_id: "m1-golden-flow-v1".into(),
            required_os_build: 19_045,
            required_runtime_profile: "whisper-large-v3-turbo-q5-vulkan".into(),
            required_sessions: 100,
            required_codex_sessions: 50,
            required_vscode_sessions: 50,
            minimum_short_sessions: 10,
            minimum_medium_sessions: 30,
            minimum_long_sessions: 50,
            maximum_irrecoverable_results: 0,
            maximum_intent_changed_sessions: 0,
            maximum_corrupt_audio_sessions: 0,
            minimum_quick_review_sessions: 51,
            maximum_release_to_terminal_p50_ms: 1_500,
            maximum_release_to_terminal_p95_ms: 2_500,
            maximum_inference_p95_ms: 1_163,
            maximum_rtf_p95: 0.0341,
            maximum_peak_ram_bytes: 727_843_636,
            maximum_peak_vram_bytes: 1_019_635_303,
            required_gates: RequiredGates {
                offline_deny_all: true,
                crash_restart: true,
                load_admission: true,
                cleanup_corpus: true,
                marker_redaction: true,
            },
        }
    }

    fn passing_run() -> GoldenFlowRun {
        GoldenFlowRun {
            schema_version: 1,
            gate_id: "m1-golden-flow-v1".into(),
            os_build: 19_045,
            runtime_profile: "whisper-large-v3-turbo-q5-vulkan".into(),
            gates: GateEvidence {
                offline_deny_all: true,
                crash_restart: true,
                load_admission: true,
                cleanup_corpus: true,
                marker_redaction: true,
            },
            sessions: (1..=100)
                .map(|ordinal| GoldenSession {
                    ordinal,
                    session_id: format!("session-{ordinal:03}"),
                    target: if ordinal <= 50 {
                        TargetFamily::Codex
                    } else {
                        TargetFamily::Vscode
                    },
                    duration_class: if ordinal <= 10 {
                        DurationClass::Short
                    } else if ordinal <= 50 {
                        DurationClass::Medium
                    } else {
                        DurationClass::Long
                    },
                    terminal_outcome: TerminalOutcome::Delivered,
                    evidence_class: DeliveryEvidence::TargetAck,
                    audio_recoverable: true,
                    text_recoverable: true,
                    quick_review: ordinal <= 60,
                    intent_changed: false,
                    corrupt_audio: false,
                    release_to_terminal_ms: 1_200,
                    inference_ms: 700,
                    audio_duration_ms: 30_000,
                    peak_ram_bytes: 700_000_000,
                    peak_vram_bytes: 1_000_000_000,
                })
                .collect(),
        }
    }

    #[test]
    fn complete_content_free_run_passes_frozen_gate() {
        let report = evaluate_golden_flow(&thresholds(), &passing_run());
        assert!(report.passed, "{:?}", report.violations);
        assert_eq!(report.counts.sessions, 100);
    }

    #[test]
    fn one_lost_result_fails_the_gate() {
        let mut run = passing_run();
        run.sessions[0].audio_recoverable = false;
        run.sessions[0].text_recoverable = false;
        let report = evaluate_golden_flow(&thresholds(), &run);
        assert!(!report.passed);
        assert_eq!(report.counts.irrecoverable, 1);
    }

    #[test]
    fn false_delivered_and_duplicate_ordinal_fail_closed() {
        let mut run = passing_run();
        run.sessions[0].evidence_class = DeliveryEvidence::TransportOnly;
        run.sessions[1].ordinal = 1;
        let report = evaluate_golden_flow(&thresholds(), &run);
        assert!(!report.passed);
        assert!(
            report
                .violations
                .iter()
                .any(|value| value.contains("strong evidence"))
        );
        assert!(
            report
                .violations
                .iter()
                .any(|value| value.contains("duplicate"))
        );
    }
}
