use crate::{Database, StorageError};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::time::Instant;

pub const CLEANUP_POLICY_VERSION: u32 = 1;
pub const CLEANUP_GLOSSARY_REVISION: u32 = 0;
pub const CLEANUP_TIMEOUT_MS: u64 = 250;
pub const CLEANUP_POLICY_HASH: &str =
    "86ae0836c57a6d166de97deba6356283a9285cdb581c114b5f5430cb96ff4d68";
pub const CLEANUP_POLICY_MANIFEST: &str = concat!(
    r#"{"policy":"meaning_preserving_cleanup","version":1,"glossary_revision":0,"#,
    r#""rules":["normalize_horizontal_whitespace","remove_isolated_fillers_v1","#,
    r#""collapse_adjacent_exact_repetitions_v1","normalize_punctuation_spacing","#,
    r#""append_terminal_period_nontechnical_v1"],"#,
    r#""protected":["negations","requirements","technical_tokens"]}"#
);

const MAX_TRANSCRIPT_BYTES: usize = 1024 * 1024;
const MAX_METRICS_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupContract {
    pub policy_version: u32,
    pub policy_hash: String,
    pub glossary_revision: u32,
}

impl CleanupContract {
    pub fn builtin() -> Self {
        Self {
            policy_version: CLEANUP_POLICY_VERSION,
            policy_hash: CLEANUP_POLICY_HASH.into(),
            glossary_revision: CLEANUP_GLOSSARY_REVISION,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRuleMetrics {
    pub whitespace_edits: u32,
    pub fillers_removed: u32,
    pub repetitions_collapsed: u32,
    pub punctuation_edits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupCandidate {
    pub content: String,
    pub contract: CleanupContract,
    pub metrics: CleanupRuleMetrics,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupEngineError {
    Failed,
    Timeout,
}

pub trait CleanupEngine {
    fn cleanup(
        &self,
        input: &str,
        contract: &CleanupContract,
    ) -> Result<CleanupCandidate, CleanupEngineError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicCleanupEngine;

impl CleanupEngine for DeterministicCleanupEngine {
    fn cleanup(
        &self,
        input: &str,
        contract: &CleanupContract,
    ) -> Result<CleanupCandidate, CleanupEngineError> {
        let started = Instant::now();
        let (content, metrics) = cleanup_text(input);
        Ok(CleanupCandidate {
            content,
            contract: contract.clone(),
            metrics,
            duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSnapshot {
    pub transcript_id: String,
    pub session_id: String,
    pub content: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupFallbackReason {
    EngineFailure,
    Timeout,
    ContractDisagreement,
    PreviousFailure,
}

impl CleanupFallbackReason {
    fn error_code(self) -> &'static str {
        match self {
            Self::EngineFailure => "cleanup_failed",
            Self::Timeout => "cleanup_timeout",
            Self::ContractDisagreement => "cleanup_contract_disagreement",
            Self::PreviousFailure => "cleanup_previous_failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupSelection {
    pub raw: TranscriptSnapshot,
    pub selected: TranscriptSnapshot,
    pub cleaned: Option<TranscriptSnapshot>,
    pub fallback_reason: Option<CleanupFallbackReason>,
}

impl CleanupSelection {
    fn cleaned(raw: TranscriptSnapshot, cleaned: TranscriptSnapshot) -> Self {
        Self {
            raw,
            selected: cleaned.clone(),
            cleaned: Some(cleaned),
            fallback_reason: None,
        }
    }

    fn raw(raw: TranscriptSnapshot, reason: CleanupFallbackReason) -> Self {
        Self {
            selected: raw.clone(),
            raw,
            cleaned: None,
            fallback_reason: Some(reason),
        }
    }
}

#[derive(Debug)]
pub enum CleanupRepositoryError {
    Storage(StorageError),
    Sqlite(rusqlite::Error),
    InvalidInput(String),
    Conflict(String),
}

impl Display for CleanupRepositoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => Display::fmt(error, formatter),
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::InvalidInput(detail) => write!(formatter, "invalid cleanup input: {detail}"),
            Self::Conflict(detail) => write!(formatter, "cleanup conflict: {detail}"),
        }
    }
}

impl std::error::Error for CleanupRepositoryError {}

impl From<StorageError> for CleanupRepositoryError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<rusqlite::Error> for CleanupRepositoryError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

pub type CleanupRepositoryResult<T> = Result<T, CleanupRepositoryError>;

pub struct CleanupRepository {
    database: Database,
}

impl CleanupRepository {
    pub fn open(path: impl AsRef<Path>) -> CleanupRepositoryResult<Self> {
        Ok(Self {
            database: Database::open(path)?,
        })
    }

    pub fn open_in_memory() -> CleanupRepositoryResult<Self> {
        Ok(Self {
            database: Database::open_in_memory()?,
        })
    }

    pub fn cleanup_raw(
        &mut self,
        raw_transcript_id: &str,
        now_ms: i64,
    ) -> CleanupRepositoryResult<CleanupSelection> {
        self.cleanup_raw_with_contract(
            raw_transcript_id,
            &CleanupContract::builtin(),
            now_ms,
            &DeterministicCleanupEngine,
        )
    }

    pub fn cleanup_next_default(
        &mut self,
        now_ms: i64,
    ) -> CleanupRepositoryResult<Option<CleanupSelection>> {
        validate_timestamp(now_ms)?;
        let contract = CleanupContract::builtin();
        let raw_id: Option<String> = self
            .database
            .connection
            .query_row(
                "SELECT raw.id
                 FROM transcript_version raw
                 WHERE raw.kind='raw'
                   AND NOT EXISTS (
                       SELECT 1 FROM cleanup_attempt attempt
                       WHERE attempt.input_transcript_id=raw.id
                         AND attempt.cleanup_profile_id IS NULL
                         AND attempt.policy_version=?1
                         AND attempt.policy_hash=?2
                         AND attempt.glossary_revision=?3
                   )
                 ORDER BY raw.created_at, raw.session_id, raw.version_no
                 LIMIT 1",
                params![
                    contract.policy_version,
                    contract.policy_hash,
                    contract.glossary_revision
                ],
                |row| row.get(0),
            )
            .optional()?;
        raw_id
            .map(|raw_id| self.cleanup_raw(&raw_id, now_ms))
            .transpose()
    }

    pub fn cleanup_raw_with_contract(
        &mut self,
        raw_transcript_id: &str,
        contract: &CleanupContract,
        now_ms: i64,
        engine: &dyn CleanupEngine,
    ) -> CleanupRepositoryResult<CleanupSelection> {
        validate_transcript_id(raw_transcript_id)?;
        validate_timestamp(now_ms)?;
        validate_builtin_contract(contract)?;
        let raw = load_transcript(&self.database.connection, raw_transcript_id, "raw")?
            .ok_or_else(|| {
                CleanupRepositoryError::InvalidInput("raw transcript not found".into())
            })?;

        if let Some(existing) = load_existing_selection(&self.database.connection, &raw, contract)?
        {
            return Ok(existing);
        }

        let candidate = match engine.cleanup(&raw.content, contract) {
            Ok(candidate) if candidate.duration_ms <= CLEANUP_TIMEOUT_MS => candidate,
            Ok(candidate) => {
                return self.persist_failure(
                    raw,
                    contract,
                    CleanupFallbackReason::Timeout,
                    candidate.metrics,
                    candidate.duration_ms,
                    now_ms,
                );
            }
            Err(CleanupEngineError::Timeout) => {
                return self.persist_failure(
                    raw,
                    contract,
                    CleanupFallbackReason::Timeout,
                    CleanupRuleMetrics::default(),
                    CLEANUP_TIMEOUT_MS,
                    now_ms,
                );
            }
            Err(CleanupEngineError::Failed) => {
                return self.persist_failure(
                    raw,
                    contract,
                    CleanupFallbackReason::EngineFailure,
                    CleanupRuleMetrics::default(),
                    0,
                    now_ms,
                );
            }
        };

        if candidate.contract != *contract
            || candidate.content.len() > MAX_TRANSCRIPT_BYTES
            || !preserves_protected_content(&raw.content, &candidate.content)
        {
            return self.persist_failure(
                raw,
                contract,
                CleanupFallbackReason::ContractDisagreement,
                candidate.metrics,
                candidate.duration_ms,
                now_ms,
            );
        }
        self.persist_success(raw, contract, candidate, now_ms)
    }

    fn persist_success(
        &mut self,
        raw: TranscriptSnapshot,
        contract: &CleanupContract,
        candidate: CleanupCandidate,
        now_ms: i64,
    ) -> CleanupRepositoryResult<CleanupSelection> {
        let ids = cleanup_ids(&raw.transcript_id, contract);
        let metrics = PersistedMetrics {
            duration_ms: candidate.duration_ms,
            disagreement: false,
            rules: candidate.metrics,
        };
        let metrics_json = serialize_metrics(&metrics)?;
        let content_hash = sha256_hex(candidate.content.as_bytes());
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load_existing_selection(&transaction, &raw, contract)? {
            transaction.rollback()?;
            return Ok(existing);
        }
        transaction.execute(
            "INSERT INTO cleanup_attempt(
                id,session_id,input_transcript_id,cleanup_profile_id,policy_version,policy_hash,
                glossary_revision,status,started_at,completed_at,error_code,metrics)
             VALUES(?1,?2,?3,NULL,?4,?5,?6,'succeeded',?7,?7,NULL,?8)",
            params![
                ids.attempt_id,
                raw.session_id,
                raw.transcript_id,
                contract.policy_version,
                contract.policy_hash,
                contract.glossary_revision,
                now_ms,
                metrics_json
            ],
        )?;
        transaction.execute(
            "INSERT INTO transcript_version(
                id,session_id,kind,version_no,content,content_hash,
                source_cleanup_attempt_id,created_at)
             VALUES(?1,?2,'cleaned',
                (SELECT COALESCE(MAX(version_no),0)+1 FROM transcript_version
                 WHERE session_id=?2 AND kind='cleaned'),?3,?4,?5,?6)",
            params![
                ids.cleaned_id,
                raw.session_id,
                candidate.content,
                content_hash,
                ids.attempt_id,
                now_ms
            ],
        )?;
        transaction.commit()?;
        let cleaned = TranscriptSnapshot {
            transcript_id: ids.cleaned_id,
            session_id: raw.session_id.clone(),
            content: candidate.content,
            content_hash,
        };
        Ok(CleanupSelection::cleaned(raw, cleaned))
    }

    fn persist_failure(
        &mut self,
        raw: TranscriptSnapshot,
        contract: &CleanupContract,
        reason: CleanupFallbackReason,
        rule_metrics: CleanupRuleMetrics,
        duration_ms: u64,
        now_ms: i64,
    ) -> CleanupRepositoryResult<CleanupSelection> {
        let ids = cleanup_ids(&raw.transcript_id, contract);
        let metrics = PersistedMetrics {
            duration_ms,
            disagreement: reason == CleanupFallbackReason::ContractDisagreement,
            rules: rule_metrics,
        };
        let metrics_json = serialize_metrics(&metrics)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load_existing_selection(&transaction, &raw, contract)? {
            transaction.rollback()?;
            return Ok(existing);
        }
        transaction.execute(
            "INSERT INTO cleanup_attempt(
                id,session_id,input_transcript_id,cleanup_profile_id,policy_version,policy_hash,
                glossary_revision,status,started_at,completed_at,error_code,metrics)
             VALUES(?1,?2,?3,NULL,?4,?5,?6,'failed',?7,?7,?8,?9)",
            params![
                ids.attempt_id,
                raw.session_id,
                raw.transcript_id,
                contract.policy_version,
                contract.policy_hash,
                contract.glossary_revision,
                now_ms,
                reason.error_code(),
                metrics_json
            ],
        )?;
        if reason == CleanupFallbackReason::ContractDisagreement {
            let metadata = serde_json::to_string(&DisagreementDiagnostic {
                disagreement: true,
                policy_version: contract.policy_version,
                policy_hash: &contract.policy_hash,
                glossary_revision: contract.glossary_revision,
                reason: reason.error_code(),
            })
            .map_err(|error| CleanupRepositoryError::InvalidInput(error.to_string()))?;
            transaction.execute(
                "INSERT INTO diagnostic_event(
                    id,session_id,component,event_type,duration_ms,error_code,metadata,occurred_at)
                 VALUES(?1,?2,'cleanup','cleanup_disagreement',?3,?4,?5,?6)",
                params![
                    ids.diagnostic_id,
                    raw.session_id,
                    i64::try_from(duration_ms).unwrap_or(i64::MAX),
                    reason.error_code(),
                    metadata,
                    now_ms
                ],
            )?;
        }
        transaction.commit()?;
        Ok(CleanupSelection::raw(raw, reason))
    }
}

#[derive(Serialize)]
struct PersistedMetrics {
    duration_ms: u64,
    disagreement: bool,
    rules: CleanupRuleMetrics,
}

#[derive(Serialize)]
struct DisagreementDiagnostic<'a> {
    disagreement: bool,
    policy_version: u32,
    policy_hash: &'a str,
    glossary_revision: u32,
    reason: &'static str,
}

struct CleanupIds {
    attempt_id: String,
    cleaned_id: String,
    diagnostic_id: String,
}

fn cleanup_ids(raw_id: &str, contract: &CleanupContract) -> CleanupIds {
    let identity = format!(
        "{}\0{}\0{}\0{}",
        raw_id, contract.policy_version, contract.policy_hash, contract.glossary_revision
    );
    let digest = sha256_hex(identity.as_bytes());
    let key = &digest[..32];
    CleanupIds {
        attempt_id: format!("cleanup-{key}"),
        cleaned_id: format!("cleaned-{key}"),
        diagnostic_id: format!("cleanup-disagreement-{key}"),
    }
}

fn load_existing_selection(
    connection: &Connection,
    raw: &TranscriptSnapshot,
    contract: &CleanupContract,
) -> CleanupRepositoryResult<Option<CleanupSelection>> {
    let attempt: Option<(String, String, Option<String>)> = connection
        .query_row(
            "SELECT id,status,error_code FROM cleanup_attempt
             WHERE input_transcript_id=?1 AND cleanup_profile_id IS NULL
               AND policy_version=?2 AND policy_hash=?3 AND glossary_revision=?4",
            params![
                raw.transcript_id,
                contract.policy_version,
                contract.policy_hash,
                contract.glossary_revision
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((attempt_id, status, error_code)) = attempt else {
        return Ok(None);
    };
    if status == "succeeded" {
        let cleaned = connection
            .query_row(
                "SELECT id,session_id,content,content_hash FROM transcript_version
                 WHERE kind='cleaned' AND source_cleanup_attempt_id=?1",
                [attempt_id],
                |row| {
                    Ok(TranscriptSnapshot {
                        transcript_id: row.get(0)?,
                        session_id: row.get(1)?,
                        content: row.get(2)?,
                        content_hash: row.get(3)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| {
                CleanupRepositoryError::Conflict(
                    "successful cleanup attempt has no immutable cleaned transcript".into(),
                )
            })?;
        return Ok(Some(CleanupSelection::cleaned(raw.clone(), cleaned)));
    }
    let reason = match error_code.as_deref() {
        Some("cleanup_timeout") => CleanupFallbackReason::Timeout,
        Some("cleanup_contract_disagreement") => CleanupFallbackReason::ContractDisagreement,
        Some("cleanup_failed") => CleanupFallbackReason::EngineFailure,
        _ => CleanupFallbackReason::PreviousFailure,
    };
    Ok(Some(CleanupSelection::raw(raw.clone(), reason)))
}

fn load_transcript(
    connection: &Connection,
    transcript_id: &str,
    kind: &str,
) -> CleanupRepositoryResult<Option<TranscriptSnapshot>> {
    Ok(connection
        .query_row(
            "SELECT id,session_id,content,content_hash FROM transcript_version
             WHERE id=?1 AND kind=?2",
            params![transcript_id, kind],
            |row| {
                Ok(TranscriptSnapshot {
                    transcript_id: row.get(0)?,
                    session_id: row.get(1)?,
                    content: row.get(2)?,
                    content_hash: row.get(3)?,
                })
            },
        )
        .optional()?)
}

fn validate_builtin_contract(contract: &CleanupContract) -> CleanupRepositoryResult<()> {
    let builtin = CleanupContract::builtin();
    if contract != &builtin {
        return Err(CleanupRepositoryError::InvalidInput(
            "policy version/hash or glossary revision does not match built-in policy".into(),
        ));
    }
    let computed = sha256_hex(CLEANUP_POLICY_MANIFEST.as_bytes());
    if computed != CLEANUP_POLICY_HASH {
        return Err(CleanupRepositoryError::Conflict(
            "compiled cleanup policy hash is inconsistent".into(),
        ));
    }
    Ok(())
}

fn validate_transcript_id(value: &str) -> CleanupRepositoryResult<()> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(CleanupRepositoryError::InvalidInput(
            "invalid raw transcript id".into(),
        ));
    }
    Ok(())
}

fn validate_timestamp(value: i64) -> CleanupRepositoryResult<()> {
    if value < 0 {
        return Err(CleanupRepositoryError::InvalidInput(
            "timestamp must be non-negative".into(),
        ));
    }
    Ok(())
}

fn serialize_metrics(metrics: &PersistedMetrics) -> CleanupRepositoryResult<String> {
    let json = serde_json::to_string(metrics)
        .map_err(|error| CleanupRepositoryError::InvalidInput(error.to_string()))?;
    if json.len() > MAX_METRICS_BYTES {
        return Err(CleanupRepositoryError::InvalidInput(
            "cleanup metrics are too large".into(),
        ));
    }
    Ok(json)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn cleanup_text(input: &str) -> (String, CleanupRuleMetrics) {
    let mut metrics = CleanupRuleMetrics::default();
    let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized != input {
        metrics.whitespace_edits = 1;
    }

    let mut without_fillers = Vec::new();
    for token in normalized.split(' ') {
        if removable_filler(token) {
            metrics.fillers_removed = metrics.fillers_removed.saturating_add(1);
        } else if !token.is_empty() {
            without_fillers.push(token);
        }
    }

    let mut deduplicated: Vec<&str> = Vec::new();
    for token in without_fillers {
        let repeated = deduplicated.last().is_some_and(|previous| {
            is_simple_word(previous)
                && is_simple_word(token)
                && previous.to_lowercase() == token.to_lowercase()
                && !is_protected_word(token)
                && !is_technical_token(token)
        });
        if repeated {
            metrics.repetitions_collapsed = metrics.repetitions_collapsed.saturating_add(1);
        } else {
            deduplicated.push(token);
        }
    }

    let joined = deduplicated.join(" ");
    let mut punctuated = String::with_capacity(joined.len().saturating_add(1));
    for character in joined.chars() {
        if matches!(character, ',' | '.' | '!' | '?' | ';' | ':') && punctuated.ends_with(' ') {
            punctuated.pop();
            metrics.punctuation_edits = metrics.punctuation_edits.saturating_add(1);
        }
        punctuated.push(character);
    }
    if punctuated.chars().last().is_some_and(char::is_alphanumeric)
        && punctuated
            .split_whitespace()
            .last()
            .is_some_and(|token| !is_technical_token(token))
    {
        punctuated.push('.');
        metrics.punctuation_edits = metrics.punctuation_edits.saturating_add(1);
    }
    (punctuated, metrics)
}

fn removable_filler(token: &str) -> bool {
    let core = lexical_core(token);
    let explicitly_isolated = token.ends_with(',') || token.ends_with(';');
    explicitly_isolated
        && matches!(
            core.to_lowercase().as_str(),
            "\u{44d}\u{43c}" | "\u{44d}\u{44d}" | "\u{44d}\u{44d}\u{44d}" | "um" | "uh"
        )
}
fn lexical_core(token: &str) -> &str {
    token.trim_matches(|character: char| {
        matches!(
            character,
            ',' | '.' | '!' | '?' | ';' | ':' | '"' | '\'' | '(' | ')' | '<' | '>'
        )
    })
}

fn is_simple_word(token: &str) -> bool {
    !token.is_empty() && token.chars().all(char::is_alphabetic)
}

fn is_protected_word(token: &str) -> bool {
    matches!(
        lexical_core(token).to_lowercase().as_str(),
        "\u{43d}\u{435}"
            | "\u{43d}\u{438}"
            | "\u{43d}\u{435}\u{442}"
            | "\u{43d}\u{438}\u{43a}\u{43e}\u{433}\u{434}\u{430}"
            | "\u{43d}\u{435}\u{43b}\u{44c}\u{437}\u{44f}"
            | "\u{431}\u{435}\u{437}"
            | "\u{43d}\u{443}\u{436}\u{43d}\u{43e}"
            | "\u{43d}\u{430}\u{434}\u{43e}"
            | "\u{434}\u{43e}\u{43b}\u{436}\u{435}\u{43d}"
            | "\u{434}\u{43e}\u{43b}\u{436}\u{43d}\u{430}"
            | "\u{434}\u{43e}\u{43b}\u{436}\u{43d}\u{43e}"
            | "\u{434}\u{43e}\u{43b}\u{436}\u{43d}\u{44b}"
            | "\u{442}\u{440}\u{435}\u{431}\u{443}\u{435}\u{442}\u{441}\u{44f}"
            | "\u{442}\u{440}\u{435}\u{431}\u{443}\u{44e}"
            | "\u{437}\u{430}\u{43f}\u{440}\u{435}\u{442}\u{438}\u{442}\u{44c}"
            | "no"
            | "not"
            | "never"
            | "without"
            | "must"
            | "should"
            | "required"
            | "require"
            | "don't"
            | "dont"
    )
}
fn is_technical_token(token: &str) -> bool {
    let trimmed = token.trim_matches(|character: char| {
        matches!(
            character,
            ',' | ';' | '!' | '?' | ':' | '"' | '\'' | '(' | ')' | '<' | '>'
        )
    });
    let lowercase = trimmed.to_ascii_lowercase();
    let known = matches!(
        lowercase.trim_end_matches('.'),
        "api"
            | "sql"
            | "rust"
            | "typescript"
            | "javascript"
            | "cargo"
            | "npm"
            | "pnpm"
            | "git"
            | "sqlite"
            | "json"
            | "ndjson"
            | "wav"
            | "asr"
            | "cli"
    );
    let uppercase = trimmed
        .bytes()
        .filter(|byte| byte.is_ascii_uppercase())
        .count();
    known
        || trimmed.starts_with("--")
        || (trimmed.starts_with('-') && trimmed.len() > 1)
        || trimmed.starts_with('/')
        || trimmed.contains(":\\")
        || trimmed.contains(":/")
        || trimmed.contains("::")
        || trimmed.contains("=>")
        || trimmed.contains('_')
        || trimmed.contains('{')
        || trimmed.contains('}')
        || trimmed.contains('[')
        || trimmed.contains(']')
        || trimmed.ends_with(".rs")
        || trimmed.ends_with(".ts")
        || trimmed.ends_with(".tsx")
        || trimmed.ends_with(".sql")
        || uppercase >= 2
}

fn preserves_protected_content(input: &str, output: &str) -> bool {
    protected_words(input) == protected_words(output)
        && technical_tokens(input) == technical_tokens(output)
}

fn protected_words(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|token| is_protected_word(token))
        .map(|token| lexical_core(token).to_lowercase())
        .collect()
}

fn technical_tokens(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter_map(|token| {
            let core = token.trim_matches(|character: char| {
                matches!(
                    character,
                    ',' | ';' | '!' | '?' | ':' | '"' | '\'' | '(' | ')' | '<' | '>'
                )
            });
            let core = if let Some(without_period) = core.strip_suffix('.') {
                if is_technical_token(without_period) {
                    without_period
                } else {
                    core
                }
            } else {
                core
            };
            is_technical_token(core).then(|| core.to_owned())
        })
        .collect()
}
