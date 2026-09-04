ALTER TABLE app_configuration
    ADD COLUMN archive_directory TEXT
    CHECK (archive_directory IS NULL OR length(archive_directory) BETWEEN 3 AND 1024);

DROP TRIGGER app_configuration_snapshot_immutable;

CREATE TRIGGER app_configuration_snapshot_immutable
BEFORE UPDATE OF schema_version, config_version, hotkey_binding, microphone_device_id,
    active_runtime_profile_id, active_cleanup_profile_id, startup_enabled,
    warmup_enabled, diagnostic_mode, archive_directory, created_at
ON app_configuration
BEGIN
    SELECT RAISE(ABORT, 'app_configuration snapshot fields are immutable');
END;
