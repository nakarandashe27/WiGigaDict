CREATE TABLE audio_commit_intent (
    commit_id TEXT PRIMARY KEY CHECK (length(commit_id) BETWEEN 1 AND 128),
    session_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL UNIQUE,
    final_storage_key TEXT NOT NULL UNIQUE CHECK (length(final_storage_key) > 0),
    runtime_profile_id TEXT NOT NULL,
    asr_attempt_id TEXT NOT NULL UNIQUE,
    asr_idempotency_key TEXT NOT NULL UNIQUE CHECK (length(asr_idempotency_key) > 0),
    prepare_event_id TEXT NOT NULL UNIQUE,
    finalizing_event_id TEXT NOT NULL UNIQUE,
    finalized_event_id TEXT NOT NULL UNIQUE,
    expected_finalizing_state_version INTEGER NOT NULL CHECK (
        expected_finalizing_state_version > 0
    ),
    sample_rate_hz INTEGER NOT NULL CHECK (sample_rate_hz BETWEEN 8000 AND 192000),
    channels INTEGER NOT NULL CHECK (channels BETWEEN 1 AND 8),
    bits_per_sample INTEGER NOT NULL CHECK (bits_per_sample = 16),
    checkpoint_state TEXT NOT NULL CHECK (checkpoint_state IN (
        'prepared', 'finalizing', 'file_promoted', 'committed', 'recovery', 'corrupt'
    )),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (artifact_id, session_id),
    FOREIGN KEY (commit_id) REFERENCES audio_artifact(commit_id) ON DELETE CASCADE,
    FOREIGN KEY (artifact_id, session_id)
        REFERENCES audio_artifact(id, session_id) ON DELETE CASCADE,
    FOREIGN KEY (session_id) REFERENCES dictation_session(id) ON DELETE CASCADE,
    FOREIGN KEY (runtime_profile_id) REFERENCES runtime_profile(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX ix_audio_commit_intent_checkpoint
    ON audio_commit_intent (checkpoint_state, created_at);