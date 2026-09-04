use crate::{Database, StorageError};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityLevel {
    Untrusted,
    Low,
    Medium,
    High,
    System,
    Unknown,
}

impl IntegrityLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::System => "system",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> DeliveryRepositoryResult<Self> {
        match value {
            "untrusted" => Ok(Self::Untrusted),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "system" => Ok(Self::System),
            "unknown" => Ok(Self::Unknown),
            _ => Err(DeliveryRepositoryError::Invariant(
                "invalid target integrity level".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshotInput {
    pub snapshot_id: String,
    pub process_identity: String,
    pub process_id: u32,
    pub thread_id: u32,
    pub window_handle: String,
    pub window_class: String,
    pub control_class: String,
    pub process_version: String,
    pub integrity_level: IntegrityLevel,
    pub integrity_rid: u32,
    pub os_build: u32,
    pub captured_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub process_identity: String,
    pub process_id: u32,
    pub thread_id: u32,
    pub window_handle: String,
    pub window_class: String,
    pub control_class: String,
    pub process_version: String,
    pub integrity_level: IntegrityLevel,
    pub integrity_rid: u32,
    pub os_build: u32,
    pub captured_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMethod {
    Unicode,
    SendInput,
    Clipboard,
}

impl DeliveryMethod {
    pub const ORDER: [Self; 3] = [Self::Unicode, Self::SendInput, Self::Clipboard];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unicode => "unicode",
            Self::SendInput => "send_input",
            Self::Clipboard => "clipboard",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceClass {
    TargetAck,
    CertifiedTransport,
    TransportOnly,
    None,
}

impl EvidenceClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TargetAck => "target_ack",
            Self::CertifiedTransport => "certified_transport",
            Self::TransportOnly => "transport_only",
            Self::None => "none",
        }
    }

    pub fn confirms_delivery(self) -> bool {
        matches!(self, Self::TargetAck | Self::CertifiedTransport)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptStatus {
    Delivered,
    Uncertain,
    Failed,
}

impl AttemptStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Uncertain => "uncertain",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Delivered,
    Uncertain,
    Failed,
}

impl DeliveryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Uncertain => "uncertain",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> DeliveryRepositoryResult<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "delivered" => Ok(Self::Delivered),
            "uncertain" => Ok(Self::Uncertain),
            "failed" => Ok(Self::Failed),
            _ => Err(DeliveryRepositoryError::Invariant(
                "invalid delivery status".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryOperation {
    pub operation_id: String,
    pub session_id: String,
    pub transcript_version_id: String,
    pub target: TargetSnapshot,
    pub operation_no: u32,
    pub status: DeliveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginDelivery {
    Ready(DeliveryOperation),
    Existing(DeliveryOperation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAttemptInput {
    pub attempt_id: String,
    pub method: DeliveryMethod,
    pub status: AttemptStatus,
    pub evidence_class: EvidenceClass,
    pub expected_input_units: Option<u32>,
    pub accepted_input_units: Option<u32>,
    pub foreground_before: String,
    pub foreground_after: Option<String>,
    pub target_revalidated: bool,
    pub keyboard_state_safe: Option<bool>,
    pub clipboard_set: Option<bool>,
    pub clipboard_restored: Option<bool>,
    pub started_at: i64,
    pub completed_at: i64,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityRuleRef {
    pub rule_id: String,
    pub rule_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryConclusion {
    pub evidence_class: EvidenceClass,
    pub compatibility_rule: Option<CompatibilityRuleRef>,
    pub error_code: Option<String>,
    pub completed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub operation_id: String,
    pub session_id: String,
    pub status: DeliveryStatus,
    pub evidence_class: EvidenceClass,
    pub attempt_count: u32,
    pub error_code: Option<String>,
}

#[derive(Debug)]
pub enum DeliveryRepositoryError {
    Storage(StorageError),
    Sqlite(rusqlite::Error),
    InvalidInput(String),
    Invariant(String),
}

impl Display for DeliveryRepositoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "{error}"),
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::InvalidInput(message) => write!(formatter, "invalid delivery input: {message}"),
            Self::Invariant(message) => write!(formatter, "delivery invariant failed: {message}"),
        }
    }
}

impl Error for DeliveryRepositoryError {}

impl From<StorageError> for DeliveryRepositoryError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<rusqlite::Error> for DeliveryRepositoryError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

pub type DeliveryRepositoryResult<T> = Result<T, DeliveryRepositoryError>;

type PersistedTargetRow = (
    String,
    u32,
    u32,
    String,
    String,
    String,
    String,
    String,
    u32,
    u32,
    i64,
);

pub struct DeliveryRepository {
    database: Database,
}

impl DeliveryRepository {
    pub fn open(path: impl AsRef<Path>) -> DeliveryRepositoryResult<Self> {
        Ok(Self {
            database: Database::open(path)?,
        })
    }

    pub fn open_in_memory() -> DeliveryRepositoryResult<Self> {
        Ok(Self {
            database: Database::open_in_memory()?,
        })
    }

    pub fn capture_initial_target(
        &mut self,
        session_id: &str,
        input: &TargetSnapshotInput,
    ) -> DeliveryRepositoryResult<TargetSnapshot> {
        validate_identifier(session_id, "session id")?;
        validate_target(input)?;
        self.database.connection.execute(
            "INSERT INTO target_snapshot(
                id,session_id,purpose,process_identity,process_id,window_handle,window_class,
                integrity_level,captured_at,thread_id,control_class,process_version,integrity_rid,
                os_build)
             VALUES(?1,?2,'initial',?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                input.snapshot_id,
                session_id,
                input.process_identity,
                input.process_id,
                input.window_handle,
                input.window_class,
                input.integrity_level.as_str(),
                input.captured_at,
                input.thread_id,
                input.control_class,
                input.process_version,
                input.integrity_rid,
                input.os_build
            ],
        )?;
        Ok(TargetSnapshot {
            snapshot_id: input.snapshot_id.clone(),
            session_id: session_id.to_owned(),
            process_identity: input.process_identity.clone(),
            process_id: input.process_id,
            thread_id: input.thread_id,
            window_handle: input.window_handle.clone(),
            window_class: input.window_class.clone(),
            control_class: input.control_class.clone(),
            process_version: input.process_version.clone(),
            integrity_level: input.integrity_level,
            integrity_rid: input.integrity_rid,
            os_build: input.os_build,
            captured_at: input.captured_at,
        })
    }

    pub fn begin_initial_delivery(
        &mut self,
        operation_id: &str,
        session_id: &str,
        transcript_version_id: &str,
        now_ms: i64,
    ) -> DeliveryRepositoryResult<BeginDelivery> {
        validate_identifier(operation_id, "operation id")?;
        validate_identifier(session_id, "session id")?;
        validate_identifier(transcript_version_id, "transcript version id")?;
        validate_timestamp(now_ms)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = load_system_operation(&transaction, session_id)? {
            let existing = if existing.status == DeliveryStatus::Pending {
                reconcile_interrupted(&transaction, existing, now_ms)?
            } else {
                existing
            };
            transaction.commit()?;
            return Ok(BeginDelivery::Existing(existing));
        }

        let target = load_initial_target(&transaction, session_id)?.ok_or_else(|| {
            DeliveryRepositoryError::Invariant("initial target snapshot is missing".into())
        })?;
        let transcript_session: Option<String> = transaction
            .query_row(
                "SELECT session_id FROM transcript_version
                 WHERE id=?1 AND kind IN ('raw','cleaned')",
                params![transcript_version_id],
                |row| row.get(0),
            )
            .optional()?;
        if transcript_session.as_deref() != Some(session_id) {
            return Err(DeliveryRepositoryError::Invariant(
                "selected transcript does not belong to the session".into(),
            ));
        }
        let pipeline_state: String = transaction.query_row(
            "SELECT pipeline_state FROM dictation_session WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        if pipeline_state != "processing" && pipeline_state != "ready_to_deliver" {
            return Err(DeliveryRepositoryError::Invariant(format!(
                "session cannot begin delivery from {pipeline_state}"
            )));
        }
        transaction.execute(
            "INSERT INTO delivery_operation(
                id,session_id,transcript_version_id,target_snapshot_id,operation_no,
                initiated_by,user_action_id,status,confirmation_level,compatibility_rule_id,
                compatibility_rule_version,started_at,completed_at,final_error_code)
             VALUES(?1,?2,?3,?4,1,'system',NULL,'pending','none',NULL,NULL,?5,NULL,NULL)",
            params![
                operation_id,
                session_id,
                transcript_version_id,
                target.snapshot_id,
                now_ms
            ],
        )?;
        transition_session(
            &transaction,
            session_id,
            "delivering",
            None,
            None,
            None,
            now_ms,
            "delivery_started",
            &pipeline_state,
            operation_id,
        )?;
        transaction.commit()?;
        Ok(BeginDelivery::Ready(DeliveryOperation {
            operation_id: operation_id.to_owned(),
            session_id: session_id.to_owned(),
            transcript_version_id: transcript_version_id.to_owned(),
            target,
            operation_no: 1,
            status: DeliveryStatus::Pending,
        }))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "retry identity, optimistic version, target, and timestamp are explicit safety inputs"
    )]
    pub fn begin_retry_delivery(
        &mut self,
        operation_id: &str,
        session_id: &str,
        transcript_version_id: &str,
        expected_state_version: u32,
        user_action_id: &str,
        target_input: &TargetSnapshotInput,
        now_ms: i64,
    ) -> DeliveryRepositoryResult<BeginDelivery> {
        validate_identifier(operation_id, "operation id")?;
        validate_identifier(session_id, "session id")?;
        validate_identifier(transcript_version_id, "transcript version id")?;
        validate_identifier(user_action_id, "user action id")?;
        validate_target(target_input)?;
        validate_timestamp(now_ms)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = load_operation_by_action(&transaction, session_id, user_action_id)?
        {
            let existing = if existing.status == DeliveryStatus::Pending {
                reconcile_interrupted(&transaction, existing, now_ms)?
            } else {
                existing
            };
            transaction.commit()?;
            return Ok(BeginDelivery::Existing(existing));
        }

        let transcript_session: Option<String> = transaction
            .query_row(
                "SELECT session_id FROM transcript_version
                 WHERE id=?1 AND kind IN ('raw','cleaned')",
                [transcript_version_id],
                |row| row.get(0),
            )
            .optional()?;
        if transcript_session.as_deref() != Some(session_id) {
            return Err(DeliveryRepositoryError::Invariant(
                "selected retry transcript does not belong to the session".into(),
            ));
        }
        let state: (String, Option<String>, u32) = transaction.query_row(
            "SELECT pipeline_state,outcome,state_version
             FROM dictation_session WHERE id=?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if state.2 != expected_state_version {
            return Err(DeliveryRepositoryError::Invariant(
                "retry session state version changed".into(),
            ));
        }
        if state.0 != "recovery"
            || !matches!(state.1.as_deref(), None | Some("uncertain" | "failed"))
        {
            return Err(DeliveryRepositoryError::Invariant(
                "retry requires an unresolved recovery session".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO target_snapshot(
                id,session_id,purpose,process_identity,process_id,window_handle,window_class,
                integrity_level,captured_at,thread_id,control_class,process_version,integrity_rid,
                os_build)
             VALUES(?1,?2,'retry',?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                target_input.snapshot_id,
                session_id,
                target_input.process_identity,
                target_input.process_id,
                target_input.window_handle,
                target_input.window_class,
                target_input.integrity_level.as_str(),
                target_input.captured_at,
                target_input.thread_id,
                target_input.control_class,
                target_input.process_version,
                target_input.integrity_rid,
                target_input.os_build
            ],
        )?;
        let operation_no: u32 = transaction.query_row(
            "SELECT COALESCE(MAX(operation_no),0)+1 FROM delivery_operation WHERE session_id=?1",
            [session_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO delivery_operation(
                id,session_id,transcript_version_id,target_snapshot_id,operation_no,
                initiated_by,user_action_id,status,confirmation_level,compatibility_rule_id,
                compatibility_rule_version,started_at,completed_at,final_error_code)
             VALUES(?1,?2,?3,?4,?5,'user',?6,'pending','none',NULL,NULL,?7,NULL,NULL)",
            params![
                operation_id,
                session_id,
                transcript_version_id,
                target_input.snapshot_id,
                operation_no,
                user_action_id,
                now_ms
            ],
        )?;
        transition_session(
            &transaction,
            session_id,
            "delivering",
            None,
            None,
            None,
            now_ms,
            "delivery_retry_started",
            "recovery",
            operation_id,
        )?;
        let target = load_target(&transaction, session_id, &target_input.snapshot_id)?
            .ok_or_else(|| DeliveryRepositoryError::Invariant("retry target is missing".into()))?;
        transaction.commit()?;
        Ok(BeginDelivery::Ready(DeliveryOperation {
            operation_id: operation_id.to_owned(),
            session_id: session_id.to_owned(),
            transcript_version_id: transcript_version_id.to_owned(),
            target,
            operation_no,
            status: DeliveryStatus::Pending,
        }))
    }
    pub fn append_attempt(
        &mut self,
        operation_id: &str,
        input: &DeliveryAttemptInput,
    ) -> DeliveryRepositoryResult<u32> {
        validate_identifier(operation_id, "operation id")?;
        validate_attempt(input)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status: Option<String> = transaction
            .query_row(
                "SELECT status FROM delivery_operation WHERE id=?1",
                params![operation_id],
                |row| row.get(0),
            )
            .optional()?;
        if status.as_deref() != Some("pending") {
            return Err(DeliveryRepositoryError::Invariant(
                "attempt can only be appended to a pending operation".into(),
            ));
        }
        let ordinal: u32 = transaction.query_row(
            "SELECT COALESCE(MAX(ordinal),0)+1 FROM delivery_attempt
             WHERE delivery_operation_id=?1",
            params![operation_id],
            |row| row.get(0),
        )?;
        let expected_method = DeliveryMethod::ORDER
            .get((ordinal - 1) as usize)
            .copied()
            .ok_or_else(|| {
                DeliveryRepositoryError::Invariant("delivery method ladder is exhausted".into())
            })?;
        if input.method != expected_method {
            return Err(DeliveryRepositoryError::Invariant(
                "delivery methods must follow the immutable ladder".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO delivery_attempt(
                id,delivery_operation_id,ordinal,method,status,evidence_class,
                expected_input_units,accepted_input_units,foreground_before,foreground_after,
                target_revalidated,keyboard_state_safe,clipboard_set,clipboard_restored,
                started_at,completed_at,error_code)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
            params![
                input.attempt_id,
                operation_id,
                ordinal,
                input.method.as_str(),
                input.status.as_str(),
                input.evidence_class.as_str(),
                input.expected_input_units,
                input.accepted_input_units,
                input.foreground_before,
                input.foreground_after,
                i64::from(input.target_revalidated),
                input.keyboard_state_safe.map(i64::from),
                input.clipboard_set.map(i64::from),
                input.clipboard_restored.map(i64::from),
                input.started_at,
                input.completed_at,
                input.error_code
            ],
        )?;
        transaction.commit()?;
        Ok(ordinal)
    }

    pub fn finalize(
        &mut self,
        operation_id: &str,
        conclusion: &DeliveryConclusion,
    ) -> DeliveryRepositoryResult<DeliveryReceipt> {
        validate_identifier(operation_id, "operation id")?;
        validate_timestamp(conclusion.completed_at)?;
        validate_conclusion(conclusion)?;
        let transaction = self
            .database
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let operation: Option<(String, String, i64)> = transaction
            .query_row(
                "SELECT session_id,status,started_at FROM delivery_operation WHERE id=?1",
                params![operation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((session_id, status, started_at)) = operation else {
            return Err(DeliveryRepositoryError::Invariant(
                "delivery operation is missing".into(),
            ));
        };
        if status != "pending" {
            return Err(DeliveryRepositoryError::Invariant(
                "delivery operation is already terminal".into(),
            ));
        }
        if conclusion.completed_at < started_at {
            return Err(DeliveryRepositoryError::InvalidInput(
                "completion precedes operation start".into(),
            ));
        }
        let attempt_count: u32 = transaction.query_row(
            "SELECT COUNT(*) FROM delivery_attempt WHERE delivery_operation_id=?1",
            params![operation_id],
            |row| row.get(0),
        )?;
        if attempt_count == 0 {
            return Err(DeliveryRepositoryError::Invariant(
                "delivery operation has no durable attempt evidence".into(),
            ));
        }
        let (attempt_evidence, attempt_status, attempt_error): (String, String, Option<String>) =
            transaction.query_row(
                "SELECT evidence_class,status,error_code FROM delivery_attempt
                 WHERE delivery_operation_id=?1 ORDER BY ordinal DESC LIMIT 1",
                params![operation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let status_matches = if conclusion.evidence_class.confirms_delivery() {
            attempt_status == "delivered"
        } else {
            matches!(attempt_status.as_str(), "uncertain" | "failed")
        };
        if attempt_evidence != conclusion.evidence_class.as_str()
            || !status_matches
            || attempt_error != conclusion.error_code
        {
            return Err(DeliveryRepositoryError::Invariant(
                "operation conclusion does not match the last immutable attempt".into(),
            ));
        }
        let delivery_status = if conclusion.evidence_class.confirms_delivery() {
            DeliveryStatus::Delivered
        } else {
            DeliveryStatus::Uncertain
        };
        let (rule_id, rule_version) = conclusion
            .compatibility_rule
            .as_ref()
            .map(|rule| (Some(rule.rule_id.as_str()), Some(rule.rule_version)))
            .unwrap_or((None, None));
        transaction.execute(
            "UPDATE delivery_operation
             SET status=?2,confirmation_level=?3,compatibility_rule_id=?4,
                 compatibility_rule_version=?5,completed_at=?6,final_error_code=?7
             WHERE id=?1 AND status='pending'",
            params![
                operation_id,
                delivery_status.as_str(),
                conclusion.evidence_class.as_str(),
                rule_id,
                rule_version,
                conclusion.completed_at,
                conclusion.error_code
            ],
        )?;
        let (pipeline_state, outcome, delivered_at, retention_expires_at, event_type) =
            match delivery_status {
                DeliveryStatus::Delivered => (
                    "done",
                    Some("delivered"),
                    Some(conclusion.completed_at),
                    Some(conclusion.completed_at + crate::DEFAULT_DELIVERED_RETENTION_MS),
                    "delivery_confirmed",
                ),
                DeliveryStatus::Uncertain => (
                    "recovery",
                    Some("uncertain"),
                    None,
                    None,
                    "delivery_uncertain",
                ),
                DeliveryStatus::Pending | DeliveryStatus::Failed => unreachable!(),
            };
        transition_session(
            &transaction,
            &session_id,
            pipeline_state,
            outcome,
            delivered_at,
            retention_expires_at,
            conclusion.completed_at,
            event_type,
            "delivering",
            operation_id,
        )?;
        transaction.commit()?;
        Ok(DeliveryReceipt {
            operation_id: operation_id.to_owned(),
            session_id,
            status: delivery_status,
            evidence_class: conclusion.evidence_class,
            attempt_count,
            error_code: conclusion.error_code.clone(),
        })
    }
}

fn validate_identifier(value: &str, label: &str) -> DeliveryRepositoryResult<()> {
    if value.is_empty() || value.len() > 256 {
        return Err(DeliveryRepositoryError::InvalidInput(format!(
            "{label} must contain 1..256 bytes"
        )));
    }
    Ok(())
}

fn validate_timestamp(value: i64) -> DeliveryRepositoryResult<()> {
    if value < 0 {
        return Err(DeliveryRepositoryError::InvalidInput(
            "timestamp must be non-negative".into(),
        ));
    }
    Ok(())
}

fn validate_target(input: &TargetSnapshotInput) -> DeliveryRepositoryResult<()> {
    validate_identifier(&input.snapshot_id, "snapshot id")?;
    validate_identifier(&input.process_identity, "process identity")?;
    validate_identifier(&input.window_handle, "window handle")?;
    validate_identifier(&input.window_class, "window class")?;
    validate_identifier(&input.control_class, "control class")?;
    validate_identifier(&input.process_version, "process version")?;
    validate_timestamp(input.captured_at)?;
    if input.process_id == 0 || input.thread_id == 0 {
        return Err(DeliveryRepositoryError::InvalidInput(
            "target process and thread ids must be non-zero".into(),
        ));
    }
    Ok(())
}

fn validate_attempt(input: &DeliveryAttemptInput) -> DeliveryRepositoryResult<()> {
    validate_identifier(&input.attempt_id, "attempt id")?;
    validate_identifier(&input.foreground_before, "foreground before")?;
    if let Some(value) = input.foreground_after.as_deref() {
        validate_identifier(value, "foreground after")?;
    }
    validate_timestamp(input.started_at)?;
    validate_timestamp(input.completed_at)?;
    if input.completed_at < input.started_at {
        return Err(DeliveryRepositoryError::InvalidInput(
            "attempt completion precedes start".into(),
        ));
    }
    if input
        .accepted_input_units
        .zip(input.expected_input_units)
        .is_some_and(|(accepted, expected)| accepted > expected)
    {
        return Err(DeliveryRepositoryError::InvalidInput(
            "accepted input units exceed expected units".into(),
        ));
    }
    if input.status == AttemptStatus::Delivered && !input.evidence_class.confirms_delivery() {
        return Err(DeliveryRepositoryError::Invariant(
            "delivered attempt lacks confirming evidence".into(),
        ));
    }
    if input.evidence_class.confirms_delivery()
        && (input.status != AttemptStatus::Delivered
            || input.expected_input_units == Some(0)
            || input.expected_input_units != input.accepted_input_units
            || !input.target_revalidated
            || input.error_code.is_some()
            || (input.clipboard_set == Some(true) && input.clipboard_restored != Some(true)))
    {
        return Err(DeliveryRepositoryError::Invariant(
            "confirming evidence requires complete, revalidated, error-free transport".into(),
        ));
    }
    if input.evidence_class == EvidenceClass::TransportOnly
        && (input.status != AttemptStatus::Uncertain
            || input.expected_input_units == Some(0)
            || input.expected_input_units != input.accepted_input_units)
    {
        return Err(DeliveryRepositoryError::Invariant(
            "transport-only evidence requires complete ambiguous transport".into(),
        ));
    }
    if input
        .error_code
        .as_ref()
        .is_some_and(|value| value.is_empty())
    {
        return Err(DeliveryRepositoryError::InvalidInput(
            "error code cannot be empty".into(),
        ));
    }
    Ok(())
}

fn validate_conclusion(conclusion: &DeliveryConclusion) -> DeliveryRepositoryResult<()> {
    match (
        conclusion.evidence_class,
        conclusion.compatibility_rule.as_ref(),
    ) {
        (EvidenceClass::CertifiedTransport, Some(rule)) => {
            validate_identifier(&rule.rule_id, "compatibility rule id")?;
            if rule.rule_version == 0 {
                return Err(DeliveryRepositoryError::InvalidInput(
                    "compatibility rule version must be positive".into(),
                ));
            }
        }
        (EvidenceClass::CertifiedTransport, None) => {
            return Err(DeliveryRepositoryError::Invariant(
                "certified transport requires a compatibility rule".into(),
            ));
        }
        (_, Some(_)) => {
            return Err(DeliveryRepositoryError::Invariant(
                "compatibility rule is only valid for certified transport".into(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn load_system_operation(
    transaction: &Transaction<'_>,
    session_id: &str,
) -> DeliveryRepositoryResult<Option<DeliveryOperation>> {
    let row: Option<(String, String, String, u32, String)> = transaction
        .query_row(
            "SELECT id,transcript_version_id,target_snapshot_id,operation_no,status
             FROM delivery_operation
             WHERE session_id=?1 AND initiated_by='system'
             ORDER BY operation_no LIMIT 1",
            params![session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(operation_id, transcript_version_id, target_snapshot_id, operation_no, status)| {
            let target =
                load_target(transaction, session_id, &target_snapshot_id)?.ok_or_else(|| {
                    DeliveryRepositoryError::Invariant("operation target is missing".into())
                })?;
            Ok(DeliveryOperation {
                operation_id,
                session_id: session_id.to_owned(),
                transcript_version_id,
                target,
                operation_no,
                status: DeliveryStatus::parse(&status)?,
            })
        },
    )
    .transpose()
}

fn load_operation_by_action(
    transaction: &Transaction<'_>,
    session_id: &str,
    user_action_id: &str,
) -> DeliveryRepositoryResult<Option<DeliveryOperation>> {
    let row: Option<(String, String, String, u32, String)> = transaction
        .query_row(
            "SELECT id,transcript_version_id,target_snapshot_id,operation_no,status
             FROM delivery_operation
             WHERE session_id=?1 AND user_action_id=?2",
            params![session_id, user_action_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(operation_id, transcript_version_id, target_snapshot_id, operation_no, status)| {
            let target =
                load_target(transaction, session_id, &target_snapshot_id)?.ok_or_else(|| {
                    DeliveryRepositoryError::Invariant("operation target is missing".into())
                })?;
            Ok(DeliveryOperation {
                operation_id,
                session_id: session_id.to_owned(),
                transcript_version_id,
                target,
                operation_no,
                status: DeliveryStatus::parse(&status)?,
            })
        },
    )
    .transpose()
}
fn load_initial_target(
    transaction: &Transaction<'_>,
    session_id: &str,
) -> DeliveryRepositoryResult<Option<TargetSnapshot>> {
    let id: Option<String> = transaction
        .query_row(
            "SELECT id FROM target_snapshot WHERE session_id=?1 AND purpose='initial'",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    id.map(|id| load_target(transaction, session_id, &id))
        .transpose()
        .map(Option::flatten)
}

fn load_target(
    transaction: &Transaction<'_>,
    session_id: &str,
    snapshot_id: &str,
) -> DeliveryRepositoryResult<Option<TargetSnapshot>> {
    let row: Option<PersistedTargetRow> = transaction
        .query_row(
            "SELECT process_identity,process_id,thread_id,window_handle,window_class,
                    control_class,process_version,integrity_level,integrity_rid,os_build,captured_at
             FROM target_snapshot WHERE id=?1 AND session_id=?2",
            params![snapshot_id, session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(
            process_identity,
            process_id,
            thread_id,
            window_handle,
            window_class,
            control_class,
            process_version,
            integrity_level,
            integrity_rid,
            os_build,
            captured_at,
        )| {
            Ok(TargetSnapshot {
                snapshot_id: snapshot_id.to_owned(),
                session_id: session_id.to_owned(),
                process_identity,
                process_id,
                thread_id,
                window_handle,
                window_class,
                control_class,
                process_version,
                integrity_level: IntegrityLevel::parse(&integrity_level)?,
                integrity_rid,
                os_build,
                captured_at,
            })
        },
    )
    .transpose()
}

fn reconcile_interrupted(
    transaction: &Transaction<'_>,
    mut operation: DeliveryOperation,
    now_ms: i64,
) -> DeliveryRepositoryResult<DeliveryOperation> {
    transaction.execute(
        "UPDATE delivery_operation
         SET status='uncertain',confirmation_level='none',completed_at=?2,
             final_error_code='delivery_interrupted'
         WHERE id=?1 AND status='pending'",
        params![operation.operation_id, now_ms],
    )?;
    transition_session(
        transaction,
        &operation.session_id,
        "recovery",
        Some("uncertain"),
        None,
        None,
        now_ms,
        "delivery_interrupted",
        "delivering",
        &operation.operation_id,
    )?;
    operation.status = DeliveryStatus::Uncertain;
    Ok(operation)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the explicit state transition contract is a safety boundary"
)]
fn transition_session(
    transaction: &Transaction<'_>,
    session_id: &str,
    pipeline_state: &str,
    outcome: Option<&str>,
    delivered_at: Option<i64>,
    retention_expires_at: Option<i64>,
    now_ms: i64,
    event_type: &str,
    expected_from: &str,
    operation_id: &str,
) -> DeliveryRepositoryResult<()> {
    let changed = transaction.execute(
        "UPDATE dictation_session
         SET pipeline_state=?2,outcome=?3,delivered_at=?4,retention_expires_at=?5,
             last_error_code=CASE WHEN ?2='recovery' THEN ?6 ELSE NULL END,
             state_version=state_version+1,updated_at=?7
         WHERE id=?1 AND pipeline_state=?8",
        params![
            session_id,
            pipeline_state,
            outcome,
            delivered_at,
            retention_expires_at,
            event_type,
            now_ms,
            expected_from
        ],
    )?;
    if changed != 1 {
        return Err(DeliveryRepositoryError::Invariant(format!(
            "session did not transition from {expected_from}"
        )));
    }
    let sequence_no: u32 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence_no),0)+1 FROM session_event WHERE session_id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let event_id = format!("{operation_id}-{event_type}");
    transaction.execute(
        "INSERT INTO session_event(
            id,session_id,sequence_no,event_type,from_state,to_state,source,reason_code,
            metadata,occurred_at)
         VALUES(?1,?2,?3,?4,?5,?6,'system',NULL,'{}',?7)",
        params![
            event_id,
            session_id,
            sequence_no,
            event_type,
            expected_from,
            pipeline_state,
            now_ms
        ],
    )?;
    Ok(())
}
