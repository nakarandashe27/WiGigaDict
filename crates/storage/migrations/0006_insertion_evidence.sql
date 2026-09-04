ALTER TABLE target_snapshot ADD COLUMN thread_id INTEGER NOT NULL DEFAULT 0 CHECK (thread_id >= 0);
ALTER TABLE target_snapshot ADD COLUMN control_class TEXT NOT NULL DEFAULT 'unknown' CHECK (length(control_class) > 0);
ALTER TABLE target_snapshot ADD COLUMN process_version TEXT NOT NULL DEFAULT 'unknown' CHECK (length(process_version) > 0);
ALTER TABLE target_snapshot ADD COLUMN integrity_rid INTEGER NOT NULL DEFAULT 0 CHECK (integrity_rid >= 0);
ALTER TABLE target_snapshot ADD COLUMN os_build INTEGER NOT NULL DEFAULT 0 CHECK (os_build >= 0);

CREATE TRIGGER trg_delivery_attempt_immutable
BEFORE UPDATE ON delivery_attempt
BEGIN
    SELECT RAISE(ABORT, 'delivery_attempt is immutable');
END;

CREATE TRIGGER trg_delivery_operation_terminal_immutable
BEFORE UPDATE ON delivery_operation
WHEN OLD.status <> 'pending'
BEGIN
    SELECT RAISE(ABORT, 'terminal delivery_operation is immutable');
END;

CREATE TRIGGER trg_delivery_attempt_terminal_only
BEFORE INSERT ON delivery_attempt
WHEN NEW.status = 'pending' OR NEW.completed_at IS NULL
BEGIN
    SELECT RAISE(ABORT, 'delivery attempts must be persisted after completion');
END;
