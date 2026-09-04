CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    name TEXT NOT NULL UNIQUE CHECK (length(name) > 0),
    checksum_sha256 TEXT NOT NULL CHECK (length(checksum_sha256) = 64),
    applied_at INTEGER NOT NULL CHECK (applied_at >= 0)
) STRICT;

CREATE TABLE dictation_session (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    pipeline_state TEXT NOT NULL CHECK (pipeline_state IN (
        'created', 'recording', 'finalizing', 'processing',
        'ready_to_deliver', 'delivering', 'done', 'recovery',
        'failed', 'cancelled'
    )),
    state_version INTEGER NOT NULL DEFAULT 0 CHECK (state_version >= 0),
    outcome TEXT CHECK (outcome IS NULL OR outcome IN (
        'delivered', 'uncertain', 'failed', 'cancelled', 'resolved'
    )),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    finalized_at INTEGER CHECK (finalized_at IS NULL OR finalized_at >= started_at),
    delivered_at INTEGER CHECK (delivered_at IS NULL OR delivered_at >= started_at),
    resolved_at INTEGER CHECK (resolved_at IS NULL OR resolved_at >= started_at),
    pinned_at INTEGER CHECK (pinned_at IS NULL OR pinned_at >= started_at),
    retention_expires_at INTEGER CHECK (
        retention_expires_at IS NULL OR retention_expires_at >= started_at
    ),
    last_error_code TEXT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (outcome <> 'delivered' OR delivered_at IS NOT NULL),
    CHECK (pipeline_state <> 'done' OR outcome = 'delivered')
) STRICT;

CREATE UNIQUE INDEX ux_dictation_session_one_recording
    ON dictation_session ((1))
    WHERE pipeline_state = 'recording';

CREATE TABLE target_snapshot (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    session_id TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (purpose IN ('initial', 'retry')),
    process_identity TEXT NOT NULL CHECK (length(process_identity) > 0),
    process_id INTEGER NOT NULL CHECK (process_id > 0),
    window_handle TEXT NOT NULL CHECK (length(window_handle) > 0),
    window_class TEXT NOT NULL CHECK (length(window_class) > 0),
    integrity_level TEXT NOT NULL CHECK (integrity_level IN (
        'untrusted', 'low', 'medium', 'high', 'system', 'unknown'
    )),
    captured_at INTEGER NOT NULL CHECK (captured_at >= 0),
    UNIQUE (id, session_id),
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_target_snapshot_initial
    ON target_snapshot (session_id)
    WHERE purpose = 'initial';

CREATE TABLE audio_artifact (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    session_id TEXT NOT NULL,
    commit_id TEXT NOT NULL UNIQUE CHECK (length(commit_id) > 0),
    staging_storage_key TEXT NOT NULL CHECK (length(staging_storage_key) > 0),
    storage_key TEXT UNIQUE,
    format TEXT NOT NULL CHECK (length(format) > 0),
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    reserved_byte_size INTEGER NOT NULL CHECK (reserved_byte_size >= 0),
    byte_size INTEGER CHECK (byte_size IS NULL OR byte_size >= 0),
    content_hash TEXT,
    artifact_state TEXT NOT NULL CHECK (artifact_state IN (
        'writing', 'finalized', 'corrupt', 'deleted'
    )),
    last_verified_at INTEGER CHECK (last_verified_at IS NULL OR last_verified_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    retention_expires_at INTEGER CHECK (
        retention_expires_at IS NULL OR retention_expires_at >= created_at
    ),
    UNIQUE (id, session_id),
    CHECK ((storage_key IS NULL) = (byte_size IS NULL)),
    CHECK ((storage_key IS NULL) = (content_hash IS NULL)),
    CHECK (artifact_state <> 'finalized' OR storage_key IS NOT NULL),
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE session_event (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    session_id TEXT NOT NULL,
    sequence_no INTEGER NOT NULL CHECK (sequence_no > 0),
    event_type TEXT NOT NULL CHECK (length(event_type) > 0),
    from_state TEXT,
    to_state TEXT,
    source TEXT NOT NULL CHECK (source IN ('user', 'system', 'windows')),
    reason_code TEXT,
    metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    UNIQUE (session_id, sequence_no),
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE model_package (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    engine_family TEXT NOT NULL CHECK (length(engine_family) > 0),
    model_name TEXT NOT NULL CHECK (length(model_name) > 0),
    model_version TEXT NOT NULL CHECK (length(model_version) > 0),
    source_uri TEXT NOT NULL CHECK (length(source_uri) > 0),
    license_id TEXT NOT NULL CHECK (length(license_id) > 0),
    expected_size INTEGER NOT NULL CHECK (expected_size > 0),
    checksum_algorithm TEXT NOT NULL CHECK (checksum_algorithm IN ('sha256')),
    checksum TEXT NOT NULL CHECK (length(checksum) = 64),
    storage_key TEXT NOT NULL CHECK (length(storage_key) > 0),
    install_state TEXT NOT NULL CHECK (install_state IN (
        'absent', 'downloading', 'verifying', 'installed', 'corrupt', 'failed'
    )),
    installed_at INTEGER CHECK (installed_at IS NULL OR installed_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (engine_family, model_name, model_version, checksum_algorithm, checksum),
    CHECK (install_state <> 'installed' OR installed_at IS NOT NULL)
) STRICT;

CREATE TABLE runtime_profile (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    profile_version INTEGER NOT NULL CHECK (profile_version > 0),
    model_package_id TEXT NOT NULL,
    adapter_type TEXT NOT NULL CHECK (length(adapter_type) > 0),
    adapter_version TEXT NOT NULL CHECK (length(adapter_version) > 0),
    device_kind TEXT NOT NULL CHECK (device_kind IN ('cpu', 'vulkan', 'directml')),
    device_id TEXT,
    settings TEXT NOT NULL CHECK (json_valid(settings)),
    settings_hash TEXT NOT NULL CHECK (length(settings_hash) = 64),
    health_state TEXT NOT NULL CHECK (health_state IN ('unknown', 'healthy', 'unhealthy')),
    last_health_at INTEGER CHECK (last_health_at IS NULL OR last_health_at >= 0),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (model_package_id, profile_version, adapter_type, device_kind, settings_hash),
    FOREIGN KEY (model_package_id) REFERENCES model_package(id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE model_install_job (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    model_package_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'queued', 'downloading', 'verifying', 'installing',
        'succeeded', 'failed', 'cancelled'
    )),
    bytes_downloaded INTEGER NOT NULL DEFAULT 0 CHECK (bytes_downloaded >= 0),
    total_bytes INTEGER NOT NULL CHECK (total_bytes > 0),
    resume_token TEXT,
    partial_storage_key TEXT NOT NULL CHECK (length(partial_storage_key) > 0),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= started_at),
    completed_at INTEGER CHECK (completed_at IS NULL OR completed_at >= started_at),
    error_code TEXT,
    CHECK (bytes_downloaded <= total_bytes),
    CHECK ((state IN ('succeeded', 'failed', 'cancelled')) = (completed_at IS NOT NULL)),
    FOREIGN KEY (model_package_id) REFERENCES model_package(id) ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX ux_model_install_job_active
    ON model_install_job (model_package_id)
    WHERE state IN ('queued', 'downloading', 'verifying', 'installing');

CREATE TABLE cleanup_profile (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    profile_key TEXT NOT NULL CHECK (length(profile_key) > 0),
    name TEXT NOT NULL CHECK (length(name) > 0),
    profile_version INTEGER NOT NULL CHECK (profile_version > 0),
    method TEXT NOT NULL CHECK (length(method) > 0),
    settings TEXT NOT NULL CHECK (json_valid(settings)),
    settings_hash TEXT NOT NULL CHECK (length(settings_hash) = 64),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    superseded_at INTEGER CHECK (superseded_at IS NULL OR superseded_at >= created_at),
    UNIQUE (profile_key, profile_version)
) STRICT;

CREATE TABLE app_profile (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    display_name TEXT NOT NULL CHECK (length(display_name) > 0),
    process_identity TEXT NOT NULL UNIQUE CHECK (length(process_identity) > 0),
    cleanup_profile_id TEXT,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    FOREIGN KEY (cleanup_profile_id) REFERENCES cleanup_profile(id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE glossary_entry (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    entry_key TEXT NOT NULL CHECK (length(entry_key) > 0),
    entry_version INTEGER NOT NULL CHECK (entry_version > 0),
    glossary_revision INTEGER NOT NULL CHECK (glossary_revision > 0),
    spoken_form TEXT NOT NULL CHECK (length(spoken_form) > 0),
    preferred_form TEXT NOT NULL CHECK (length(preferred_form) > 0),
    aliases TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(aliases)),
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('global', 'app', 'mode')),
    app_profile_id TEXT,
    mode TEXT CHECK (mode IS NULL OR mode IN ('dictation', 'notetaker')),
    case_policy TEXT NOT NULL CHECK (case_policy IN ('preserve', 'preferred', 'insensitive')),
    priority INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    superseded_at INTEGER CHECK (superseded_at IS NULL OR superseded_at >= created_at),
    UNIQUE (entry_key, entry_version),
    CHECK (
        (scope_kind = 'global' AND app_profile_id IS NULL AND mode IS NULL) OR
        (scope_kind = 'app' AND app_profile_id IS NOT NULL AND mode IS NULL) OR
        (scope_kind = 'mode' AND app_profile_id IS NULL AND mode IS NOT NULL)
    ),
    FOREIGN KEY (app_profile_id) REFERENCES app_profile(id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE asr_attempt (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    session_id TEXT NOT NULL,
    audio_artifact_id TEXT NOT NULL,
    runtime_profile_id TEXT NOT NULL,
    attempt_no INTEGER NOT NULL CHECK (attempt_no > 0),
    idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) > 0),
    language_hint TEXT,
    status TEXT NOT NULL CHECK (status IN (
        'queued', 'leased', 'running', 'succeeded', 'failed', 'cancelled'
    )),
    queued_at INTEGER NOT NULL CHECK (queued_at >= 0),
    lease_owner TEXT,
    lease_expires_at INTEGER CHECK (lease_expires_at IS NULL OR lease_expires_at >= queued_at),
    heartbeat_at INTEGER CHECK (heartbeat_at IS NULL OR heartbeat_at >= queued_at),
    started_at INTEGER CHECK (started_at IS NULL OR started_at >= queued_at),
    completed_at INTEGER CHECK (
        completed_at IS NULL OR (started_at IS NOT NULL AND completed_at >= started_at)
    ),
    error_code TEXT,
    metrics TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metrics)),
    UNIQUE (id, session_id),
    UNIQUE (session_id, attempt_no),
    CHECK ((status IN ('succeeded', 'failed', 'cancelled')) = (completed_at IS NOT NULL)),
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE,
    FOREIGN KEY (audio_artifact_id, session_id)
        REFERENCES audio_artifact(id, session_id) ON DELETE RESTRICT,
    FOREIGN KEY (runtime_profile_id) REFERENCES runtime_profile(id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE transcript_version (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('raw', 'cleaned')),
    version_no INTEGER NOT NULL CHECK (version_no > 0),
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL CHECK (length(content_hash) = 64),
    source_asr_attempt_id TEXT,
    source_cleanup_attempt_id TEXT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    UNIQUE (id, session_id),
    UNIQUE (session_id, kind, version_no),
    CHECK (
        (kind = 'raw' AND source_asr_attempt_id IS NOT NULL AND source_cleanup_attempt_id IS NULL) OR
        (kind = 'cleaned' AND source_asr_attempt_id IS NULL AND source_cleanup_attempt_id IS NOT NULL)
    ),
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE,
    FOREIGN KEY (source_asr_attempt_id, session_id)
        REFERENCES asr_attempt(id, session_id) ON DELETE RESTRICT,
    FOREIGN KEY (source_cleanup_attempt_id, session_id)
        REFERENCES cleanup_attempt(id, session_id) ON DELETE RESTRICT
) STRICT;

CREATE UNIQUE INDEX ux_transcript_version_asr_source
    ON transcript_version (source_asr_attempt_id)
    WHERE source_asr_attempt_id IS NOT NULL;

CREATE UNIQUE INDEX ux_transcript_version_cleanup_source
    ON transcript_version (source_cleanup_attempt_id)
    WHERE source_cleanup_attempt_id IS NOT NULL;

CREATE TABLE cleanup_attempt (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    session_id TEXT NOT NULL,
    input_transcript_id TEXT NOT NULL,
    cleanup_profile_id TEXT,
    policy_version INTEGER NOT NULL CHECK (policy_version > 0),
    policy_hash TEXT NOT NULL CHECK (length(policy_hash) = 64),
    glossary_revision INTEGER NOT NULL CHECK (glossary_revision >= 0),
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'cancelled')),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    completed_at INTEGER CHECK (completed_at IS NULL OR completed_at >= started_at),
    error_code TEXT,
    metrics TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metrics)),
    UNIQUE (id, session_id),
    CHECK ((status IN ('succeeded', 'failed', 'cancelled')) = (completed_at IS NOT NULL)),
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE,
    FOREIGN KEY (input_transcript_id, session_id)
        REFERENCES transcript_version(id, session_id) ON DELETE RESTRICT,
    FOREIGN KEY (cleanup_profile_id) REFERENCES cleanup_profile(id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE delivery_operation (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    session_id TEXT NOT NULL,
    transcript_version_id TEXT NOT NULL,
    target_snapshot_id TEXT NOT NULL,
    operation_no INTEGER NOT NULL CHECK (operation_no > 0),
    initiated_by TEXT NOT NULL CHECK (initiated_by IN ('user', 'system')),
    user_action_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'delivered', 'uncertain', 'failed')),
    confirmation_level TEXT NOT NULL CHECK (confirmation_level IN (
        'target_ack', 'certified_transport', 'transport_only', 'none'
    )),
    compatibility_rule_id TEXT,
    compatibility_rule_version INTEGER CHECK (
        compatibility_rule_version IS NULL OR compatibility_rule_version > 0
    ),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    completed_at INTEGER CHECK (completed_at IS NULL OR completed_at >= started_at),
    final_error_code TEXT,
    UNIQUE (id, session_id),
    UNIQUE (session_id, operation_no),
    CHECK ((compatibility_rule_id IS NULL) = (compatibility_rule_version IS NULL)),
    CHECK (
        confirmation_level <> 'certified_transport' OR compatibility_rule_id IS NOT NULL
    ),
    CHECK (status <> 'delivered' OR confirmation_level IN (
        'target_ack', 'certified_transport'
    )),
    CHECK ((status IN ('delivered', 'uncertain', 'failed')) = (completed_at IS NOT NULL)),
    CHECK (initiated_by = 'user' OR user_action_id IS NULL),
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE,
    FOREIGN KEY (transcript_version_id, session_id)
        REFERENCES transcript_version(id, session_id) ON DELETE RESTRICT,
    FOREIGN KEY (target_snapshot_id, session_id)
        REFERENCES target_snapshot(id, session_id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE delivery_attempt (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    delivery_operation_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    method TEXT NOT NULL CHECK (method IN ('unicode', 'send_input', 'clipboard')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'delivered', 'uncertain', 'failed')),
    evidence_class TEXT NOT NULL CHECK (evidence_class IN (
        'target_ack', 'certified_transport', 'transport_only', 'none'
    )),
    expected_input_units INTEGER CHECK (expected_input_units IS NULL OR expected_input_units >= 0),
    accepted_input_units INTEGER CHECK (accepted_input_units IS NULL OR accepted_input_units >= 0),
    foreground_before TEXT NOT NULL CHECK (length(foreground_before) > 0),
    foreground_after TEXT CHECK (foreground_after IS NULL OR length(foreground_after) > 0),
    target_revalidated INTEGER NOT NULL CHECK (target_revalidated IN (0, 1)),
    keyboard_state_safe INTEGER CHECK (keyboard_state_safe IS NULL OR keyboard_state_safe IN (0, 1)),
    clipboard_set INTEGER CHECK (clipboard_set IS NULL OR clipboard_set IN (0, 1)),
    clipboard_restored INTEGER CHECK (clipboard_restored IS NULL OR clipboard_restored IN (0, 1)),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    completed_at INTEGER CHECK (completed_at IS NULL OR completed_at >= started_at),
    error_code TEXT,
    UNIQUE (delivery_operation_id, ordinal),
    UNIQUE (delivery_operation_id, method),
    CHECK (accepted_input_units IS NULL OR expected_input_units IS NOT NULL),
    CHECK (accepted_input_units IS NULL OR accepted_input_units <= expected_input_units),
    CHECK (status <> 'delivered' OR evidence_class IN ('target_ack', 'certified_transport')),
    CHECK ((status IN ('delivered', 'uncertain', 'failed')) = (completed_at IS NOT NULL)),
    FOREIGN KEY (delivery_operation_id) REFERENCES delivery_operation(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE app_configuration (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    config_version INTEGER NOT NULL UNIQUE CHECK (config_version > 0),
    is_active INTEGER NOT NULL CHECK (is_active IN (0, 1)),
    hotkey_binding TEXT NOT NULL CHECK (length(hotkey_binding) > 0),
    microphone_device_id TEXT,
    active_runtime_profile_id TEXT,
    active_cleanup_profile_id TEXT,
    startup_enabled INTEGER NOT NULL CHECK (startup_enabled IN (0, 1)),
    warmup_enabled INTEGER NOT NULL CHECK (warmup_enabled IN (0, 1)),
    diagnostic_mode INTEGER NOT NULL CHECK (diagnostic_mode IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    activated_at INTEGER CHECK (activated_at IS NULL OR activated_at >= created_at),
    superseded_at INTEGER CHECK (superseded_at IS NULL OR superseded_at >= created_at),
    CHECK (is_active = 0 OR (activated_at IS NOT NULL AND superseded_at IS NULL)),
    FOREIGN KEY (active_runtime_profile_id) REFERENCES runtime_profile(id) ON DELETE RESTRICT,
    FOREIGN KEY (active_cleanup_profile_id) REFERENCES cleanup_profile(id) ON DELETE RESTRICT
) STRICT;

CREATE UNIQUE INDEX ux_app_configuration_active
    ON app_configuration ((1))
    WHERE is_active = 1;

CREATE TABLE diagnostic_event (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    session_id TEXT,
    component TEXT NOT NULL CHECK (length(component) > 0),
    event_type TEXT NOT NULL CHECK (length(event_type) > 0),
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    error_code TEXT,
    metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE maintenance_run (
    id TEXT PRIMARY KEY CHECK (length(id) > 0),
    run_type TEXT NOT NULL CHECK (length(run_type) > 0),
    cutoff_at INTEGER NOT NULL CHECK (cutoff_at >= 0),
    cursor TEXT,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'cancelled')),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    completed_at INTEGER CHECK (completed_at IS NULL OR completed_at >= started_at),
    error_code TEXT,
    CHECK ((status IN ('succeeded', 'failed', 'cancelled')) = (completed_at IS NOT NULL))
) STRICT;

CREATE TRIGGER target_snapshot_immutable
BEFORE UPDATE ON target_snapshot
BEGIN
    SELECT RAISE(ABORT, 'target_snapshot is immutable');
END;

CREATE TRIGGER session_event_append_only
BEFORE UPDATE ON session_event
BEGIN
    SELECT RAISE(ABORT, 'session_event is append-only');
END;

CREATE TRIGGER transcript_version_immutable
BEFORE UPDATE ON transcript_version
BEGIN
    SELECT RAISE(ABORT, 'transcript_version is immutable');
END;

CREATE TRIGGER cleanup_profile_version_immutable
BEFORE UPDATE OF profile_key, name, profile_version, method, settings, settings_hash, created_at
ON cleanup_profile
BEGIN
    SELECT RAISE(ABORT, 'cleanup_profile version fields are immutable');
END;

CREATE TRIGGER app_configuration_snapshot_immutable
BEFORE UPDATE OF schema_version, config_version, hotkey_binding, microphone_device_id,
    active_runtime_profile_id, active_cleanup_profile_id, startup_enabled,
    warmup_enabled, diagnostic_mode, created_at
ON app_configuration
BEGIN
    SELECT RAISE(ABORT, 'app_configuration snapshot fields are immutable');
END;

CREATE TRIGGER glossary_entry_version_immutable
BEFORE UPDATE OF entry_key, entry_version, glossary_revision, spoken_form, preferred_form,
    aliases, scope_kind, app_profile_id, mode, case_policy, priority, created_at
ON glossary_entry
BEGIN
    SELECT RAISE(ABORT, 'glossary_entry version fields are immutable');
END;

CREATE INDEX ix_audio_artifact_session ON audio_artifact (session_id);
CREATE INDEX ix_session_event_session_time ON session_event (session_id, occurred_at);
CREATE INDEX ix_asr_attempt_queue ON asr_attempt (status, queued_at);
CREATE INDEX ix_cleanup_attempt_session ON cleanup_attempt (session_id, started_at);
CREATE INDEX ix_delivery_operation_session ON delivery_operation (session_id, operation_no);
CREATE INDEX ix_diagnostic_event_time ON diagnostic_event (occurred_at);
CREATE INDEX ix_maintenance_run_status ON maintenance_run (status, started_at);