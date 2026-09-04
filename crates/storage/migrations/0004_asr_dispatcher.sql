ALTER TABLE asr_attempt
    ADD COLUMN lease_generation INTEGER NOT NULL DEFAULT 0 CHECK (lease_generation >= 0);

UPDATE asr_attempt
SET status = 'queued', lease_owner = NULL, lease_expires_at = NULL, heartbeat_at = NULL
WHERE status IN ('leased', 'running');

CREATE UNIQUE INDEX ux_asr_attempt_single_active_lease
    ON asr_attempt ((1))
    WHERE status IN ('leased', 'running');

CREATE INDEX ix_asr_attempt_fifo ON asr_attempt (status, queued_at, session_id, attempt_no);
