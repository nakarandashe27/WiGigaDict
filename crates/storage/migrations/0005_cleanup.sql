CREATE UNIQUE INDEX ux_cleanup_attempt_builtin_contract
    ON cleanup_attempt (input_transcript_id, policy_version, policy_hash, glossary_revision)
    WHERE cleanup_profile_id IS NULL;

CREATE TRIGGER cleanup_attempt_contract_immutable
BEFORE UPDATE OF session_id, input_transcript_id, cleanup_profile_id,
    policy_version, policy_hash, glossary_revision, started_at
ON cleanup_attempt
BEGIN
    SELECT RAISE(ABORT, 'cleanup attempt contract is immutable');
END;

CREATE TRIGGER cleanup_attempt_metrics_no_content_insert
BEFORE INSERT ON cleanup_attempt
WHEN EXISTS (
    SELECT 1 FROM json_tree(NEW.metrics)
    WHERE lower(key) IN (
        'content', 'text', 'transcript_content', 'raw_content',
        'cleaned_content', 'input_text', 'output_text'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'cleanup metrics must not contain transcript content');
END;

CREATE TRIGGER cleanup_attempt_metrics_no_content_update
BEFORE UPDATE OF metrics ON cleanup_attempt
WHEN EXISTS (
    SELECT 1 FROM json_tree(NEW.metrics)
    WHERE lower(key) IN (
        'content', 'text', 'transcript_content', 'raw_content',
        'cleaned_content', 'input_text', 'output_text'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'cleanup metrics must not contain transcript content');
END;

CREATE TRIGGER diagnostic_event_no_content_insert
BEFORE INSERT ON diagnostic_event
WHEN EXISTS (
    SELECT 1 FROM json_tree(NEW.metadata)
    WHERE lower(key) IN (
        'content', 'text', 'transcript_content', 'raw_content',
        'cleaned_content', 'input_text', 'output_text'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'diagnostics must not contain transcript content');
END;

CREATE TRIGGER diagnostic_event_no_content_update
BEFORE UPDATE OF metadata ON diagnostic_event
WHEN EXISTS (
    SELECT 1 FROM json_tree(NEW.metadata)
    WHERE lower(key) IN (
        'content', 'text', 'transcript_content', 'raw_content',
        'cleaned_content', 'input_text', 'output_text'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'diagnostics must not contain transcript content');
END;
