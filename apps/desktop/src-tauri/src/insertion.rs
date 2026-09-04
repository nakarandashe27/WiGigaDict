use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use wigigadict_storage::{
    AttemptStatus, BeginDelivery, CompatibilityRuleRef, DeliveryAttemptInput, DeliveryConclusion,
    DeliveryMethod, DeliveryOperation, DeliveryReceipt, DeliveryRepository,
    DeliveryRepositoryError, DeliveryStatus, EvidenceClass, TargetSnapshot, TargetSnapshotInput,
    TranscriptSnapshot,
};

pub const COMPATIBILITY_REGISTRY_MANIFEST: &str =
    r#"{"schema_version":1,"registry_version":1,"windows_11_supported":false,"rules":[]}"#;
pub const COMPATIBILITY_REGISTRY_HASH: &str =
    "a9bb8488b04d6d9c93f582e29a129d396ff988415af835e4c7037873b0e6db8e";
pub const COMPATIBILITY_REGISTRY_VERSION: u32 = 1;
const MAX_INSERTION_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertionFailure {
    ClipboardBusy,
    ClipboardRestoreFailed,
    ElevatedTarget,
    EmptyTranscript,
    FocusChanged,
    InputPartiallyAccepted,
    InvalidText,
    KeyboardStateUnsafe,
    TargetMissing,
    UnsupportedCharacter,
    UnsupportedWindowsVersion,
    Win32CallFailed,
}

impl InsertionFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::ClipboardBusy => "clipboard_busy",
            Self::ClipboardRestoreFailed => "clipboard_restore_failed",
            Self::ElevatedTarget => "elevated_target",
            Self::EmptyTranscript => "empty_transcript",
            Self::FocusChanged => "focus_changed",
            Self::InputPartiallyAccepted => "input_partially_accepted",
            Self::InvalidText => "invalid_insertion_text",
            Self::KeyboardStateUnsafe => "keyboard_state_unsafe",
            Self::TargetMissing => "target_missing",
            Self::UnsupportedCharacter => "unsupported_character",
            Self::UnsupportedWindowsVersion => "unsupported_windows_version",
            Self::Win32CallFailed => "win32_call_failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportAttempt {
    pub expected_units: u32,
    pub accepted_units: u32,
    pub target_acknowledged: bool,
    pub keyboard_state_safe: Option<bool>,
    pub clipboard_set: Option<bool>,
    pub clipboard_restored: Option<bool>,
    pub failure: Option<InsertionFailure>,
}

impl TransportAttempt {
    pub fn zero(failure: InsertionFailure) -> Self {
        Self {
            expected_units: 0,
            accepted_units: 0,
            target_acknowledged: false,
            keyboard_state_safe: None,
            clipboard_set: None,
            clipboard_restored: None,
            failure: Some(failure),
        }
    }
}

pub trait InsertionPlatform {
    fn revalidate(&mut self, target: &TargetSnapshot) -> Result<String, InsertionFailure>;
    fn insert(
        &mut self,
        method: DeliveryMethod,
        text: &str,
        target: &TargetSnapshot,
    ) -> TransportAttempt;
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityRule {
    pub id: String,
    pub version: u32,
    pub active: bool,
    pub evidence_hash: String,
    pub os_build_min: u32,
    pub os_build_max: u32,
    pub process_identity: String,
    pub process_version_min: String,
    pub process_version_max: String,
    pub window_class: String,
    pub control_class: String,
    pub method: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryManifest {
    schema_version: u32,
    registry_version: u32,
    windows_11_supported: bool,
    rules: Vec<CompatibilityRule>,
}

#[derive(Debug, Clone)]
pub struct CompatibilityRegistry {
    registry_version: u32,
    windows_11_supported: bool,
    rules: Vec<CompatibilityRule>,
}

impl CompatibilityRegistry {
    pub fn builtin() -> Result<Self, InsertionError> {
        if hex_sha256(COMPATIBILITY_REGISTRY_MANIFEST.as_bytes()) != COMPATIBILITY_REGISTRY_HASH {
            return Err(InsertionError::Registry(
                "built-in compatibility registry hash mismatch".into(),
            ));
        }
        let manifest: RegistryManifest = serde_json::from_str(COMPATIBILITY_REGISTRY_MANIFEST)
            .map_err(|_| InsertionError::Registry("registry manifest is invalid".into()))?;
        if manifest.schema_version != 1
            || manifest.registry_version != COMPATIBILITY_REGISTRY_VERSION
        {
            return Err(InsertionError::Registry(
                "registry schema or version is unsupported".into(),
            ));
        }
        Self::from_manifest(manifest)
    }

    #[cfg(test)]
    pub fn from_rules(
        registry_version: u32,
        windows_11_supported: bool,
        rules: Vec<CompatibilityRule>,
    ) -> Result<Self, InsertionError> {
        Self::from_manifest(RegistryManifest {
            schema_version: 1,
            registry_version,
            windows_11_supported,
            rules,
        })
    }

    fn from_manifest(manifest: RegistryManifest) -> Result<Self, InsertionError> {
        if manifest.registry_version == 0 {
            return Err(InsertionError::Registry(
                "registry version must be positive".into(),
            ));
        }
        for (index, rule) in manifest.rules.iter().enumerate() {
            validate_rule(rule)?;
            if manifest.rules[..index]
                .iter()
                .any(|other| other.id == rule.id && other.version == rule.version)
            {
                return Err(InsertionError::Registry(
                    "duplicate compatibility rule id/version".into(),
                ));
            }
        }
        Ok(Self {
            registry_version: manifest.registry_version,
            windows_11_supported: manifest.windows_11_supported,
            rules: manifest.rules,
        })
    }

    pub fn registry_version(&self) -> u32 {
        self.registry_version
    }

    fn matching_rule(
        &self,
        target: &TargetSnapshot,
        method: DeliveryMethod,
    ) -> Option<CompatibilityRuleRef> {
        if target.process_version == "unknown"
            || target.os_build == 0
            || (target.os_build >= 22_000 && !self.windows_11_supported)
        {
            return None;
        }
        self.rules
            .iter()
            .filter(|rule| {
                rule.active
                    && target.os_build >= rule.os_build_min
                    && target.os_build <= rule.os_build_max
                    && rule
                        .process_identity
                        .eq_ignore_ascii_case(&target.process_identity)
                    && version_in_range(
                        &target.process_version,
                        &rule.process_version_min,
                        &rule.process_version_max,
                    )
                    && rule.window_class == target.window_class
                    && rule.control_class == target.control_class
                    && rule.method == method.as_str()
            })
            .max_by_key(|rule| rule.version)
            .map(|rule| CompatibilityRuleRef {
                rule_id: rule.id.clone(),
                rule_version: rule.version,
            })
    }
}

fn validate_rule(rule: &CompatibilityRule) -> Result<(), InsertionError> {
    let valid_method = DeliveryMethod::ORDER
        .iter()
        .any(|method| method.as_str() == rule.method);
    let valid_range = version_in_range(
        &rule.process_version_min,
        &rule.process_version_min,
        &rule.process_version_max,
    );
    if rule.id.is_empty()
        || rule.version == 0
        || rule.evidence_hash.len() != 64
        || !rule
            .evidence_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || rule.os_build_min == 0
        || rule.os_build_min > rule.os_build_max
        || rule.process_identity.is_empty()
        || rule.window_class.is_empty()
        || rule.control_class.is_empty()
        || !valid_method
        || !valid_range
    {
        return Err(InsertionError::Registry(
            "compatibility rule is incomplete or invalid".into(),
        ));
    }
    Ok(())
}

fn parse_version(value: &str) -> Option<Vec<u32>> {
    if value.is_empty() {
        return None;
    }
    value
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

fn version_in_range(value: &str, minimum: &str, maximum: &str) -> bool {
    let (Some(mut value), Some(mut minimum), Some(mut maximum)) = (
        parse_version(value),
        parse_version(minimum),
        parse_version(maximum),
    ) else {
        return false;
    };
    let width = value.len().max(minimum.len()).max(maximum.len());
    value.resize(width, 0);
    minimum.resize(width, 0);
    maximum.resize(width, 0);
    minimum <= value && value <= maximum
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryRun {
    Completed(DeliveryReceipt),
    Existing(DeliveryStatus),
}

#[derive(Debug)]
pub enum InsertionError {
    Repository(DeliveryRepositoryError),
    Registry(String),
}

impl Display for InsertionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository(error) => write!(formatter, "{error}"),
            Self::Registry(message) => write!(formatter, "evidence registry error: {message}"),
        }
    }
}

impl std::error::Error for InsertionError {}

impl From<DeliveryRepositoryError> for InsertionError {
    fn from(value: DeliveryRepositoryError) -> Self {
        Self::Repository(value)
    }
}

pub struct InsertionCoordinator<P> {
    repository: DeliveryRepository,
    platform: P,
    registry: CompatibilityRegistry,
}

impl<P: InsertionPlatform> InsertionCoordinator<P> {
    pub fn new(
        repository: DeliveryRepository,
        platform: P,
        registry: CompatibilityRegistry,
    ) -> Self {
        Self {
            repository,
            platform,
            registry,
        }
    }

    pub fn deliver_initial(
        &mut self,
        transcript: &TranscriptSnapshot,
    ) -> Result<DeliveryRun, InsertionError> {
        let started_at = now_ms();
        let operation_id = format!("delivery-{}", Uuid::new_v4());
        let operation = match self.repository.begin_initial_delivery(
            &operation_id,
            &transcript.session_id,
            &transcript.transcript_id,
            started_at,
        )? {
            BeginDelivery::Ready(operation) => operation,
            BeginDelivery::Existing(operation) => {
                return Ok(DeliveryRun::Existing(operation.status));
            }
        };
        self.deliver_operation(transcript, operation, started_at)
    }

    pub fn deliver_retry(
        &mut self,
        transcript: &TranscriptSnapshot,
        target: &TargetSnapshotInput,
        expected_state_version: u32,
        user_action_id: &str,
    ) -> Result<DeliveryRun, InsertionError> {
        let started_at = now_ms();
        let operation_id = format!("delivery-retry-{}", Uuid::new_v4());
        let operation = match self.repository.begin_retry_delivery(
            &operation_id,
            &transcript.session_id,
            &transcript.transcript_id,
            expected_state_version,
            user_action_id,
            target,
            started_at,
        )? {
            BeginDelivery::Ready(operation) => operation,
            BeginDelivery::Existing(operation) => {
                return Ok(DeliveryRun::Existing(operation.status));
            }
        };
        self.deliver_operation(transcript, operation, started_at)
    }

    fn deliver_operation(
        &mut self,
        transcript: &TranscriptSnapshot,
        operation: DeliveryOperation,
        started_at: i64,
    ) -> Result<DeliveryRun, InsertionError> {
        if let Some(failure) = insertion_text_failure(&transcript.content) {
            self.persist_attempt(
                &operation.operation_id,
                DeliveryMethod::Unicode,
                &operation.target.window_handle,
                None,
                false,
                TransportAttempt::zero(failure),
                EvidenceClass::None,
                AttemptStatus::Uncertain,
                Some(failure),
                started_at,
            )?;
            return self.finish(
                &operation.operation_id,
                EvidenceClass::None,
                None,
                Some(failure),
            );
        }

        for method in DeliveryMethod::ORDER {
            let attempt_started = now_ms();
            let before = match self.platform.revalidate(&operation.target) {
                Ok(value) => value,
                Err(failure) => {
                    self.persist_attempt(
                        &operation.operation_id,
                        method,
                        &operation.target.window_handle,
                        None,
                        false,
                        TransportAttempt::zero(failure),
                        EvidenceClass::None,
                        AttemptStatus::Uncertain,
                        Some(failure),
                        attempt_started,
                    )?;
                    return self.finish(
                        &operation.operation_id,
                        EvidenceClass::None,
                        None,
                        Some(failure),
                    );
                }
            };

            let transport = self
                .platform
                .insert(method, &transcript.content, &operation.target);
            let accepted_any = transport.accepted_units > 0;
            let (after, after_failure) = if accepted_any {
                match self.platform.revalidate(&operation.target) {
                    Ok(value) => (Some(value), None),
                    Err(failure) => (None, Some(failure)),
                }
            } else {
                (None, None)
            };
            let decision = classify_attempt(
                &self.registry,
                &operation.target,
                method,
                &transport,
                after_failure,
            );
            let AttemptDecision {
                evidence,
                status,
                rule,
                failure,
                terminal_uncertainty,
            } = decision;
            let fallback_allowed = may_fallback(&transport, failure);
            self.persist_attempt(
                &operation.operation_id,
                method,
                &before,
                after,
                true,
                transport,
                evidence,
                status,
                failure,
                attempt_started,
            )?;

            if evidence.confirms_delivery() || accepted_any || terminal_uncertainty {
                return self.finish(&operation.operation_id, evidence, rule, failure);
            }
            if !fallback_allowed {
                return self.finish(&operation.operation_id, evidence, rule, failure);
            }
        }
        self.finish(
            &operation.operation_id,
            EvidenceClass::None,
            None,
            Some(InsertionFailure::UnsupportedCharacter),
        )
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "complete attempt evidence is intentionally explicit"
    )]
    fn persist_attempt(
        &mut self,
        operation_id: &str,
        method: DeliveryMethod,
        foreground_before: &str,
        foreground_after: Option<String>,
        target_revalidated: bool,
        transport: TransportAttempt,
        evidence_class: EvidenceClass,
        status: AttemptStatus,
        failure: Option<InsertionFailure>,
        started_at: i64,
    ) -> Result<(), InsertionError> {
        self.repository.append_attempt(
            operation_id,
            &DeliveryAttemptInput {
                attempt_id: format!("attempt-{}", Uuid::new_v4()),
                method,
                status,
                evidence_class,
                expected_input_units: Some(transport.expected_units),
                accepted_input_units: Some(transport.accepted_units),
                foreground_before: foreground_before.to_owned(),
                foreground_after,
                target_revalidated,
                keyboard_state_safe: transport.keyboard_state_safe,
                clipboard_set: transport.clipboard_set,
                clipboard_restored: transport.clipboard_restored,
                started_at,
                completed_at: now_ms().max(started_at),
                error_code: failure.map(|value| value.code().into()),
            },
        )?;
        Ok(())
    }

    fn finish(
        &mut self,
        operation_id: &str,
        evidence_class: EvidenceClass,
        compatibility_rule: Option<CompatibilityRuleRef>,
        failure: Option<InsertionFailure>,
    ) -> Result<DeliveryRun, InsertionError> {
        Ok(DeliveryRun::Completed(self.repository.finalize(
            operation_id,
            &DeliveryConclusion {
                evidence_class,
                compatibility_rule,
                error_code: failure.map(|value| value.code().into()),
                completed_at: now_ms(),
            },
        )?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttemptDecision {
    evidence: EvidenceClass,
    status: AttemptStatus,
    rule: Option<CompatibilityRuleRef>,
    failure: Option<InsertionFailure>,
    terminal_uncertainty: bool,
}

fn classify_attempt(
    registry: &CompatibilityRegistry,
    target: &TargetSnapshot,
    method: DeliveryMethod,
    transport: &TransportAttempt,
    after_failure: Option<InsertionFailure>,
) -> AttemptDecision {
    let accepted_any = transport.accepted_units > 0;
    let complete =
        transport.expected_units > 0 && transport.accepted_units == transport.expected_units;
    let restoration_failed = method == DeliveryMethod::Clipboard
        && transport.clipboard_set == Some(true)
        && transport.clipboard_restored != Some(true);
    if restoration_failed {
        return AttemptDecision {
            evidence: EvidenceClass::None,
            status: AttemptStatus::Uncertain,
            rule: None,
            failure: Some(InsertionFailure::ClipboardRestoreFailed),
            terminal_uncertainty: true,
        };
    }
    if let Some(failure) = after_failure {
        return AttemptDecision {
            evidence: EvidenceClass::None,
            status: AttemptStatus::Uncertain,
            rule: None,
            failure: Some(failure),
            terminal_uncertainty: true,
        };
    }
    if accepted_any && !complete {
        return AttemptDecision {
            evidence: EvidenceClass::None,
            status: AttemptStatus::Uncertain,
            rule: None,
            failure: Some(InsertionFailure::InputPartiallyAccepted),
            terminal_uncertainty: true,
        };
    }
    if complete && transport.target_acknowledged {
        return AttemptDecision {
            evidence: EvidenceClass::TargetAck,
            status: AttemptStatus::Delivered,
            rule: None,
            failure: None,
            terminal_uncertainty: false,
        };
    }
    if complete {
        let rule = registry.matching_rule(target, method);
        return AttemptDecision {
            evidence: if rule.is_some() {
                EvidenceClass::CertifiedTransport
            } else {
                EvidenceClass::TransportOnly
            },
            status: if rule.is_some() {
                AttemptStatus::Delivered
            } else {
                AttemptStatus::Uncertain
            },
            rule,
            failure: transport.failure,
            terminal_uncertainty: false,
        };
    }
    AttemptDecision {
        evidence: EvidenceClass::None,
        status: AttemptStatus::Failed,
        rule: None,
        failure: transport
            .failure
            .or(Some(InsertionFailure::Win32CallFailed)),
        terminal_uncertainty: false,
    }
}

fn may_fallback(transport: &TransportAttempt, failure: Option<InsertionFailure>) -> bool {
    transport.accepted_units == 0 && failure == Some(InsertionFailure::UnsupportedCharacter)
}

/// An empty (or whitespace-only) transcript is a distinct owner situation from
/// invalid text: nothing was recognized, so nothing can ever be inserted and a
/// retry is pointless. It gets its own error code so the overlay can say that
/// instead of reporting a delivery failure.
fn insertion_text_failure(text: &str) -> Option<InsertionFailure> {
    if text.trim().is_empty() {
        return Some(InsertionFailure::EmptyTranscript);
    }
    if text.len() > MAX_INSERTION_BYTES || text.chars().any(|character| character.is_control()) {
        return Some(InsertionFailure::InvalidText);
    }
    None
}

pub type WindowsInsertionCoordinator =
    InsertionCoordinator<crate::windows_insertion::WindowsInsertionPlatform>;

impl WindowsInsertionCoordinator {
    pub fn open(database_path: impl AsRef<Path>) -> Result<Self, InsertionError> {
        let registry = CompatibilityRegistry::builtin()?;
        if registry.registry_version() != COMPATIBILITY_REGISTRY_VERSION {
            return Err(InsertionError::Registry(
                "active compatibility registry version mismatch".into(),
            ));
        }
        Ok(Self::new(
            DeliveryRepository::open(database_path)?,
            crate::windows_insertion::WindowsInsertionPlatform,
            registry,
        ))
    }
}

pub fn capture_foreground_target(
    snapshot_id: String,
    captured_at: i64,
) -> Result<TargetSnapshotInput, InsertionFailure> {
    crate::windows_insertion::capture_target(snapshot_id, captured_at)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use wigigadict_storage::IntegrityLevel;

    fn target() -> TargetSnapshot {
        TargetSnapshot {
            snapshot_id: "target-1".into(),
            session_id: "session-1".into(),
            process_identity: "fixture.exe".into(),
            process_id: 10,
            thread_id: 11,
            window_handle: "0x1".into(),
            window_class: "FixtureWindow".into(),
            control_class: "Edit".into(),
            process_version: "1.2.3".into(),
            integrity_level: IntegrityLevel::Medium,
            integrity_rid: 0x2000,
            os_build: 19_045,
            captured_at: 1,
        }
    }

    fn rule() -> CompatibilityRule {
        CompatibilityRule {
            id: "win10-fixture-edit-unicode".into(),
            version: 1,
            active: true,
            evidence_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .into(),
            os_build_min: 19_041,
            os_build_max: 19_045,
            process_identity: "fixture.exe".into(),
            process_version_min: "1.2.0".into(),
            process_version_max: "1.2.9".into(),
            window_class: "FixtureWindow".into(),
            control_class: "Edit".into(),
            method: "unicode".into(),
        }
    }

    #[test]
    fn builtin_registry_is_hash_pinned_and_empty() {
        let registry = CompatibilityRegistry::builtin().unwrap();
        assert_eq!(registry.registry_version(), 1);
        assert!(
            registry
                .matching_rule(&target(), DeliveryMethod::Unicode)
                .is_none()
        );
    }

    #[test]
    fn compatibility_match_is_exact_and_windows_11_fails_closed() {
        let registry = CompatibilityRegistry::from_rules(1, false, vec![rule()]).unwrap();
        let matched = registry
            .matching_rule(&target(), DeliveryMethod::Unicode)
            .unwrap();
        assert_eq!(matched.rule_id, "win10-fixture-edit-unicode");

        let mut changed = target();
        changed.control_class = "Chrome_RenderWidgetHostHWND".into();
        assert!(
            registry
                .matching_rule(&changed, DeliveryMethod::Unicode)
                .is_none()
        );
        changed = target();
        changed.process_version = "unknown".into();
        assert!(
            registry
                .matching_rule(&changed, DeliveryMethod::Unicode)
                .is_none()
        );
        changed = target();
        changed.os_build = 26_100;
        assert!(
            registry
                .matching_rule(&changed, DeliveryMethod::Unicode)
                .is_none()
        );
    }

    #[test]
    fn evidence_matrix_never_turns_ambiguous_transport_into_success() {
        let empty = CompatibilityRegistry::builtin().unwrap();
        let full = TransportAttempt {
            expected_units: 8,
            accepted_units: 8,
            target_acknowledged: false,
            keyboard_state_safe: Some(true),
            clipboard_set: None,
            clipboard_restored: None,
            failure: None,
        };
        let transport_only =
            classify_attempt(&empty, &target(), DeliveryMethod::Unicode, &full, None);
        assert_eq!(transport_only.evidence, EvidenceClass::TransportOnly);
        assert_eq!(transport_only.status, AttemptStatus::Uncertain);

        let focus_changed = classify_attempt(
            &empty,
            &target(),
            DeliveryMethod::Unicode,
            &full,
            Some(InsertionFailure::FocusChanged),
        );
        assert_eq!(focus_changed.evidence, EvidenceClass::None);
        assert!(focus_changed.terminal_uncertainty);

        let mut partial = full.clone();
        partial.accepted_units = 6;
        let partial = classify_attempt(&empty, &target(), DeliveryMethod::Unicode, &partial, None);
        assert_eq!(
            partial.failure,
            Some(InsertionFailure::InputPartiallyAccepted)
        );
        assert_eq!(partial.status, AttemptStatus::Uncertain);

        let mut clipboard = full;
        clipboard.clipboard_set = Some(true);
        clipboard.clipboard_restored = Some(false);
        let clipboard = classify_attempt(
            &empty,
            &target(),
            DeliveryMethod::Clipboard,
            &clipboard,
            None,
        );
        assert_eq!(
            clipboard.failure,
            Some(InsertionFailure::ClipboardRestoreFailed)
        );
        assert!(clipboard.terminal_uncertainty);
    }

    #[test]
    fn ladder_only_falls_back_after_proven_zero_unsupported_input() {
        let zero_unsupported = TransportAttempt::zero(InsertionFailure::UnsupportedCharacter);
        assert!(may_fallback(
            &zero_unsupported,
            Some(InsertionFailure::UnsupportedCharacter)
        ));
        let mut partial = zero_unsupported.clone();
        partial.expected_units = 8;
        partial.accepted_units = 6;
        assert!(!may_fallback(
            &partial,
            Some(InsertionFailure::InputPartiallyAccepted)
        ));
        let mut clipboard_restore = partial;
        clipboard_restore.clipboard_set = Some(true);
        clipboard_restore.clipboard_restored = Some(false);
        assert!(!may_fallback(
            &clipboard_restore,
            Some(InsertionFailure::ClipboardRestoreFailed)
        ));
    }

    #[test]
    fn enter_and_other_control_characters_are_never_inserted() {
        assert_eq!(insertion_text_failure("safe transcript"), None);
        assert_eq!(
            insertion_text_failure("line one\nline two"),
            Some(InsertionFailure::InvalidText)
        );
        assert_eq!(
            insertion_text_failure("tab\tchanges focus"),
            Some(InsertionFailure::InvalidText)
        );
    }

    #[test]
    fn empty_transcript_is_named_instead_of_reported_as_invalid_delivery() {
        assert_eq!(
            insertion_text_failure(""),
            Some(InsertionFailure::EmptyTranscript)
        );
        assert_eq!(
            insertion_text_failure("   "),
            Some(InsertionFailure::EmptyTranscript)
        );
    }
}
