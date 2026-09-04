# ASR benchmark

This directory contains M0 Step 4 benchmark contracts, not production ASR code.

- The SAPI manifest is a deterministic smoke corpus. It validates the harness and boundary handling but must never select an engine.
- The ignored private directory is reserved for a human-recorded RU/EN technical corpus. Evidence identifies every WAV by SHA-256.
- Generated WAV and machine evidence files are ignored. Reviewed summaries may be committed.

Accepted audio is integer PCM, 16 kHz, mono, 16-bit and exactly 5, 15, 25, 30 or 60 seconds. A final marker makes truncation explicit.

The gate remains incomplete until the selection corpus covers Whisper and GigaAM on CPU/GPU, cold/warm, worker crash/restart and a clean machine, with measured incremental watts/kWh. Synthetic speech and TDP estimates are not selection evidence.
