# WiGigaDict storage

`wigigadict-storage` owns SQLite connection setup and versioned schema migrations for the M1 domain. It is intentionally independent from the desktop UI and ASR worker.

## Contract

- Persistent connections enable `foreign_keys=ON`, `journal_mode=WAL`, `synchronous=FULL`, `secure_delete=ON`, and a bounded busy timeout before migrations.
- `PRAGMA user_version` selects ordered migrations. `schema_migrations` binds each applied migration to its name and SHA-256; a future version or changed historical migration fails closed.
- `0001_initial.sql` represents exactly the 18 M1 entities from the architecture. M2 `Notetaker*` tables are not created.
- Domain relations use foreign keys, including composite same-session keys where a child must belong to the same `DictationSession`.
- Core state, XOR, uniqueness, delivery-evidence and timestamp rules are database constraints. Immutable/versioned records are guarded by update triggers.
- UI and ASR code must use future repository contracts over this crate. They must not open the database or embed SQL directly.

## Durable audio commit (Step 6)

- Migration `0002_audio_commit_intent.sql` adds a technical reconciliation ledger keyed by `commit_id`; it is not a nineteenth M1 domain entity.
- `prepare_pcm_writer` commits the session, writing artifact and intent before returning a bounded PCM S16LE mono 16 kHz writer. `write_samples` performs no SQL, hashing or flushing.
- Finalization writes a canonical WAV header, verifies the reserved byte limit, calls `FlushFileBuffers`, and promotes the same-volume `.wav.part` with write-through rename and no overwrite.
- A compare-and-swap SQLite transaction records hash/size/final markers, queues exactly one ASR attempt and marks the intent committed.
- Startup reconciliation classifies every known commit as `continue`, `recovery` or `corrupt`; incomplete artifacts are preserved, unknown files are quarantined, and repeated reconciliation is idempotent.
## Recovery, history, and retention (Step 13)

- Migration `0007_recovery_retention.sql` makes every explicit delivery `user_action_id` globally unique while preserving the 18-table M1 model.
- `RecoveryRepository` projects history directly from session/transcript/delivery rows. It never persists a second transcript copy.
- Delivered unpinned sessions receive a 15-day cutoff. Pinned, unresolved, and active sessions are excluded from retention sweep.
- Delete plans live as content-free JSON cursors in `maintenance_run`. They remove normalized staging/final/quarantine keys before the SQLite cascade, then run WAL truncate and `VACUUM`; a crash leaves a resumable running journal.
- Targeted tests cover restart projection, immutable attempts, optimistic/idempotent actions, explicit retry no-replay, retention exclusions, and byte-level absence from SQLite/WAL/SHM/PCM/`.part`/quarantine.

## Versioned application configuration (Step 14)

- `ConfigurationRepository` uses the existing immutable `app_configuration` table; no schema migration or second settings store was added.
- Default, active, and last-known-good snapshots are versioned. Updates require the expected active version and stale writers fail closed.
- Hotkey, input device, runtime, cleanup, startup, warm-up, and diagnostic intents are validated before insertion. Warm-up requires an installed/enabled/healthy runtime; cleanup cannot target a superseded contract.
- Repository tests cover immutable version progression, stale-update preservation of last-known-good, and fail-closed warm-up validation.

## Content-free diagnostics (Step 15)

- `DiagnosticStore` accepts only versioned typed events with closed enums, bounded machine-safe identifiers, monotonic sequence, and allowlisted numeric/boolean/hash metadata.
- NDJSON append is synchronized. Startup truncates only an incomplete final line; complete corruption, future schema, unknown entries, duplicate sequence, symlinks, and Windows reparse points fail closed.
- Rolling retention defaults to 30 days, 100 MiB total, 25 files, 4 MiB per file, and 16 KiB per event. Restart also expires a stale active trace without touching session artifacts.
- Deterministic bundles reparse every source, record hashes/counts/missing sources, cap output at 100 MiB, and exclude content fields by construction. Export requires a matching prepared preview id, exact confirmation, an absolute new `.wigigadiag.json` path, and atomic `.part → final` promotion.
- Six targeted tests cover marker-secret absence, rotation/retention/restart, crash-tail recovery, fail-closed schema/entry handling, preview/confirmation/no-overwrite, and ordered recovery/focus/commit failure trace.


## Verification

```powershell
cargo test -p wigigadict-storage --locked --offline
cargo clippy -p wigigadict-storage --all-targets --locked --offline -- -D warnings
```

The storage suites cover fresh creation and upgrades, schema/checksum fail-closed behavior, exact M1 domain-table membership, database constraints, durable audio checkpoints, dispatcher/cleanup/delivery repositories, Step 13 recovery/retention/deletion fault cases, Step 14 versioned configuration, and Step 15 content-free diagnostics.