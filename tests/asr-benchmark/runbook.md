# M0 Step 4 ASR benchmark runbook

This harness is isolated under `tools/asr-benchmark`. It must not be used as production dictation or Notetaker code.

## Fixed toolchain

- Rust 1.97.1 `x86_64-pc-windows-msvc` from `rust-toolchain.toml`.
- Visual Studio Build Tools 2022 plus Windows SDK, initialized by `scripts/initialize-vsenv.ps1`.
- Adapter `transcribe-rs=0.3.11` from `tools/asr-benchmark/Cargo.lock`.
- Whisper binding `whisper-rs=0.16.0` / `whisper-rs-sys=0.15.0`.
- ONNX Runtime crate `ort=2.0.0-rc.12` selected by the same lock.
- LLVM/libclang 22.1.8; `libclang.dll` SHA-256 `51fed10c43c3d31c1fe5bfe76bac60150970961e9b9b23cf014dbfcb5398bbfc`.
- Vulkan SDK 1.4.357.0.
- Ninja from the pinned Visual Studio installation and short Cargo target directory `.t`.

Do not set `WHISPER_DONT_GENERATE_BINDINGS` on Windows: the crate's pregenerated bindings target a different C ABI and fail MSVC layout assertions.

## Prepare exact inputs

```powershell
scripts/prepare-asr-smoke-corpus.ps1
scripts/prepare-whisper-benchmark-model.ps1 -Variant LargeTurboQ5
scripts/prepare-whisper-benchmark-model.ps1 -Variant LargeTurboQ8
scripts/prepare-whisper-benchmark-model.ps1 -Variant LargeV3Q5
scripts/prepare-whisper-benchmark-model.ps1 -Variant SmallQ5
scripts/prepare-gigaam-benchmark-model.ps1 -Variant Int8
```

The download scripts use exact repository revisions, resumable `.part` files, expected byte counts and SHA-256 checks. Private models, generated WAV files and raw NDJSON evidence are ignored by Git; manifests and aggregate reports are versioned.

`LargeTurboQ8` is retained for reproducible benchmark comparison only. Its boundary-safe `take-04` preflight failed the marker/WER/CER improvement gate, so do not run a full Q8 matrix or treat it as selected without new evidence.

`LargeV3Q5` is also benchmark-only. It improved matched mean WER but did not improve final-marker recovery, slightly regressed mean CER and increased latency/RAM, so its full matrix is likewise rejected without new evidence.

The SAPI corpus is only a deterministic harness smoke test. Engine selection requires ignored human recordings of the same RU/EN prompts. Preserve consent, pseudonymous speaker/run IDs, source hashes and normalized 16 kHz mono PCM hashes. Use at least three independent recordings per prompt, including more than one voice when available, so quality and latency distributions are not inferred from one take or one voice.

List capture devices without recording:

```powershell
scripts/record-asr-human-corpus.ps1 -ListDevices
```

Run capture with PowerShell 7.4 or newer. Record an interactive private take; the script first records a five-second signal preflight, fails closed below a -50 dB peak, then waits for Enter before every sample and never overwrites files:

```powershell
pwsh -File scripts/record-asr-human-corpus.ps1 `
  -SpeakerId speaker-a -Take 2 `
  -DeviceName '<exact DirectShow name from -ListDevices>' `
  -TailPaddingSeconds 3
```

Use the exact localized device name printed by `-ListDevices`; the reference take used EarPods, not a portable default. Speak the displayed preflight phrase continuously. A failed preflight creates no corpus sample. Human selection takes should use three seconds of tail padding: finish the displayed prompt naturally and remain silent during the padding. The manifest speech target and padded capture duration are stored separately in provenance. Provenance schema 3 also binds the exact manifest by repository-relative path, byte count and SHA-256; corpus ID alone is not sufficient. `0` preserves exact-duration capture for boundary experiments. Do not accept a take by file count alone: verify provenance shape, manifest hash, WAV hashes, PCM format, duration, signal level and tail silence. The rejected silent `take-01` and boundary-confounded `take-03` remain private diagnostic evidence and must not be merged into unqualified final-marker statistics.

The locally installed FFmpeg is accepted only as capture tooling for this private benchmark. The current machine's build reports `--enable-gpl`; the script records that fact and sets `production_bundle_approved=false`. It must never be copied into the product or confused with the separate reproducible LGPL FFmpeg work.

## Build environment

```powershell
. scripts/initialize-vsenv.ps1
Initialize-VsDevEnvironment
$env:CARGO_TARGET_DIR = Join-Path (Get-Location) '.t'
$env:LIBCLANG_PATH = 'C:\Program Files\LLVM\bin'
```

For Whisper Vulkan also set:

```powershell
$env:VULKAN_SDK = 'C:\VulkanSDK\1.4.357.0'
$env:CMAKE_GENERATOR = 'Ninja'
$env:CMAKE_BUILD_PARALLEL_LEVEL = '1'
$env:GGML_CCACHE = 'OFF'
$env:Path = 'C:\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;C:\VulkanSDK\1.4.357.0\Bin;' + $env:Path
$env:GGML_VK_VISIBLE_DEVICES = '1' # machine-specific; record the resulting native device log
cargo build --manifest-path tools/asr-benchmark/Cargo.toml --locked --release --features whisper-vulkan
```

The short target path is required because the native Vulkan shader sub-build exceeded the Windows compiler/PDB path limit under the default nested Cargo target.

For the current Whisper CPU fallback evidence, use the same exact Vulkan-capable worker and model as the GPU run, set a non-`gpu` profile and pass an explicit bounded thread count. The harness records `n_threads` in every Whisper evidence row; `0` preserves the upstream default and valid values are `0..=64`.

```powershell
cargo build --manifest-path tools/asr-benchmark/Cargo.toml --locked --release `
  --features whisper-vulkan
Get-FileHash .t/release/wigigadict-asr-benchmark.exe -Algorithm SHA256
.t/release/wigigadict-asr-benchmark.exe run-whisper `
  --model tests/asr-benchmark/private/models/whisper.cpp/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin `
  --audio tests/asr-benchmark/generated/en-005.wav --sample en-005 --language en `
  --profile cpu-t16 --mode cold --threads 16 --output tests/asr-benchmark/evidence/cpu.ndjson
```

The accepted local shared worker has SHA-256 `CB05D28CF8BAA67B3A593747DC5EFAD6BB03FCAFE33DA3810249B3CB6BB84177`. Its non-`gpu` profile reports `whisper.cpp-cpu`; verify both fields before accepting evidence. A fresh standalone `whisper-cpu` build regressed to 109,383 ms on a five-second human sample and timed out in controlled retries, so it is rejected pending root-cause analysis. Do not mix its partial evidence with the shared-worker matrix.

On the reference Ryzen 9 8940HX, 16 threads had the lowest stable warm p95 in the earlier synthetic sweep. Re-run the thread sweep on materially different CPUs instead of assuming that logical-processor count is optimal. The local tuning, synthetic duration and human shared-worker reports are `reports/2026-08-22-whisper-large-cpu-thread-sweep.json`, `reports/2026-08-22-whisper-large-cpu-t16-duration.json` and `reports/2026-08-22-whisper-vulkan-capable-cpu-t16-human-speaker-a-take02.json`. Clean-machine startup without manual Vulkan repair remains a gate.

For GigaAM CPU/DirectML:

```powershell
cargo build --manifest-path tools/asr-benchmark/Cargo.toml --locked --release --features gigaam-directml
```

`gigaam-directml` also contains the CPU path. The current adapter does not expose DirectML device selection; do not attribute it to a GPU without external proof.

## Run contract

For every engine, use the same immutable WAV artifacts and pass explicit `--sample`, `--language`, `--profile`, `--mode` and `--output`. Modes are exactly `cold` or `warm`. GigaAM profiles are exactly `cpu` or `gpu`; Whisper additionally permits `cpu-t1` through `cpu-t64`, and the suffix must equal `--threads`. Plain Whisper `cpu` means the upstream thread default (`--threads 0`). Cold creates a worker, loads once and measures one inference. Warm loads once, performs one unrecorded inference, then measures the next inference in the same worker.

The worker creates a new evidence file by default and refuses an existing output. For a predeclared multi-record matrix, create the first row without `--append`. Each subsequent row requires both `--append` and `--expected-existing-records N`; the worker refuses a count mismatch. Never append to an evidence path whose matrix identity or existing rows have not just been verified.

Whisper example:

```powershell
.t/release/wigigadict-asr-benchmark.exe run-whisper `
  --model tests/asr-benchmark/private/models/whisper.cpp/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin `
  --audio tests/asr-benchmark/generated/en-060.wav --sample en-060 --language en `
  --profile gpu --mode warm --output tests/asr-benchmark/evidence/example.ndjson
```

GigaAM example:

```powershell
.t/release/wigigadict-asr-benchmark.exe run-gigaam `
  --model-dir tests/asr-benchmark/private/models/gigaam-v3-onnx/322c3b29492673eb7d0b434bfa9dfb8653e34d02 `
  --quantization int8 --audio tests/asr-benchmark/generated/ru-060.wav `
  --sample ru-060 --language ru --profile cpu --mode cold `
  --output tests/asr-benchmark/evidence/example.ndjson
```

Run all 5/15/25/30/60 samples, both languages, CPU/GPU and cold/warm. Never append a new matrix to an ambiguous evidence file; use a new path and verify the expected record count. The convenience `SmokeCpu` orchestration generates a unique evidence path and refuses an explicitly supplied path that already exists.

## Analyze

```powershell
scripts/analyze-asr-benchmark.ps1 `
  -Evidence tests/asr-benchmark/evidence/example.ndjson `
  -Output tests/asr-benchmark/reports/example.json
```

The analyzer uses deterministic Unicode normalization, Levenshtein WER/CER, exact technical-token checks, terminal whole-token final-marker aliases, monotonic segment checks and nearest-rank p50/p95. It rejects arbitrary profile/mode labels, language mismatches and any ASR segment outside `audio_duration_ms`. A missing marker is reported as `final_marker_miss_count`; it is not called truncation without independent temporal evidence.

Run the focused contract tests without loading an ASR model:

```powershell
pwsh -File scripts/test-asr-benchmark-contracts.ps1
```

## Direct runtime oracle parity

Adapter parity is separate from model quality. Build the official `whisper.cpp` v1.8.3 CLI from detached commit `2eeeba56e9edd762b4b38467bab96c2517163158`, compare its source tree with the `whisper-rs-sys=0.15.0` vendor tree after deterministic CRLF-to-LF normalization, and record both tree and binary hashes. On MSVC, preserve `/EHsc`; the accepted local oracle used `-DCMAKE_CXX_FLAGS='/utf-8 /EHsc'` and Vulkan SDK 1.4.357.0.

Match the adapter's decoding parameters explicitly:

```powershell
.t/oracle-whisper-v1.8.3-build/bin/whisper-cli.exe `
  -m tests/asr-benchmark/private/models/whisper.cpp/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin `
  -f tests/asr-benchmark/generated/en-060.wav -l en -t 4 -bs 3 `
  -nth 0.2 -sns -fa -ojf -of .t/oracle-en-060 -np
```

Require exact full text, segment count and segment text. Record timestamp deltas separately; the local v1.8.3 comparison accepted at most 50 ms and observed at most 40 ms. Never interpret a shared oracle/adapter hallucination as parity success for model selection. Aggregate evidence is in `reports/2026-08-22-whisper-oracle-parity.json`; raw JSON/NDJSON remains ignored.

The current GigaAM CTC artifact came from a third-party export and is rejected. It cannot serve as an official GigaAM oracle. If another GigaAM candidate is evaluated, pin and compare against the official Salute runtime independently.

## Crash/restart

`fault-after-load` must be compiled with the engine feature. It loads the real model/runtime and then intentionally exits 86. A valid test records baseline, asserts exit 86, starts a new process on the same sample and compares text/model/audio hashes. Do not interpret this benchmark hook as durable lease recovery; that is M1 Step 10.

## NVIDIA telemetry

`scripts/measure-nvidia-asr.ps1` launches hidden worker processes, samples the exact NVIDIA index with `nvidia-smi`, records process-level incremental power/energy and peak incremental VRAM, and keeps per-process stdout/stderr in the requested log directory. Its scope includes model load and optional warmup. It does not estimate CPU power or identify the device selected by DirectML.

## Battery whole-system telemetry

`scripts/measure-battery-asr.ps1` is a benchmark-only fallback when no trustworthy CPU package sensor or external wall meter is available. It reads `root/wmi BatteryStatus.DischargeRate`, launches the worker hidden and integrates whole-system plus idle-subtracted energy. It refuses AC power, charging, zero/unknown rate, critical battery, existing output and an existing process-log directory. Check readiness first:

```powershell
scripts/measure-battery-asr.ps1 -PreflightOnly
powercfg.exe /getactivescheme
```

After the user explicitly disconnects AC and preflight reports `measurement_ready=true`, run the exact CPU profile:

```powershell
$arguments = @(
  'run-whisper',
  '--model', 'tests/asr-benchmark/private/models/whisper.cpp/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin',
  '--audio', 'tests/asr-benchmark/generated/en-060.wav',
  '--sample', 'en-060-battery', '--language', 'en',
  '--profile', 'cpu-t16', '--mode', 'warm', '--threads', '16',
  '--output', 'tests/asr-benchmark/evidence/whisper-large-cpu-t16-battery.ndjson'
)
scripts/measure-battery-asr.ps1 `
  -Binary '.t/release/wigigadict-asr-benchmark.exe' `
  -BenchmarkArguments $arguments `
  -Output 'tests/asr-benchmark/reports/2026-08-22-whisper-large-cpu-t16-battery.json' `
  -Repetitions 3 -IdleSamples 15 -PollMilliseconds 1000
```

This is battery-reported whole-system discharge, not CPU package power and not AC wall power. Microsoft documents rate as milliwatts unless the battery uses relative units. `BatteryStaticData` capability lookup fails on the reference machine, so the report preserves that limitation and cannot alone satisfy a strict CPU-package-power gate.

The reference three-run `cpu-t16` result is recorded in `reports/2026-08-22-whisper-large-cpu-t16-battery.json` with a separate immutable-source assessment in `reports/2026-08-22-whisper-large-cpu-t16-battery-quality.json`. Its measured inference RTF p50/p95 is 3.62028/3.63865, so this profile fails real-time operation on battery. Do not compare battery latency directly with AC evidence without preserving the active power scheme and battery state. Avoid status polling during measurement; the reference run preserves two brief read-only snapshots as explicit measurement noise.

## Clean-machine gate

Use a fresh supported Windows 10 VM or physical machine. Start from a clean checkout, install only the pinned repository prerequisites, prepare exact artifacts by the scripts above and run the release matrix with no Python environment, Hugging Face cache or copied native DLLs. Preserve OS/build, hardware, driver, command transcript, artifact hashes, expected record counts and dependency/license reports. If manual runtime repair is needed, the gate fails and the runbook must be corrected before retry.

The current machine is not clean-machine evidence. Windows 11 compatibility remains separately deferred and must not be inferred from this run.

## Supply-chain gate

Benchmark use is not production approval. The isolated lock has no advisory/source/bans failure, but the current production allowlist rejects `Unlicense` for Whisper bindings and `CDLA-Permissive-2.0` in ORT build tooling. Resolve policy and SBOM before moving any chain into the production sidecar.
## Boundary speech-tail validation

This benchmark-only gate detects sustained speech-like activity in the final second; it does not run ASR and does not record audio. The frozen contract is `boundary-contract.md`. Generate the ignored calibration/held-out WAV fixture, run unit tests, and require `acceptance_pass=true` before classifying existing human takes:

```powershell
scripts/prepare-asr-boundary-fixture.ps1
cargo test --manifest-path tools/asr-benchmark/Cargo.toml --bin asr-boundary-validator
cargo run --manifest-path tools/asr-benchmark/Cargo.toml --bin asr-boundary-validator -- `
  --manifest tests/asr-benchmark/generated/boundary-fixture/manifest.json `
  --output tests/asr-benchmark/reports/boundary-classifier.json
```

The ignored fixture manifest binds each WAV by SHA-256. A diagnostic human-corpus manifest uses `purpose: "diagnostic"`, `split: "diagnostic"`, and `label: "unknown"`; diagnostic reports intentionally serialize `acceptance_pass`, `legacy_correct`, and `primary_correct` as `null`. In report metrics, `primary_pass=true` means speech-like tail activity was detected. Do not relabel `primary_pass=false` as corpus independence, representativeness, or ASR quality.

Do not tune thresholds after reading human-take results. A failed held-out split requires a new preregistered contract and a newly generated held-out fixture. Keep generated/control WAVs and human manifests ignored; only sanitized JSON/Markdown reports may be versioned.
## Frozen personal-MVP selection

ADR-006 closes Step 4 for the owner's personal alpha with Whisper large-v3-turbo Q5 on Vulkan and explicit `cpu-t16` recovery fallback. No additional speaker or recording is required for that scope. Do not retune the boundary classifier or select a new model from the existing human takes. Reopen selection only for a concrete Step 16 golden-flow regression or a separately preregistered future public/multi-speaker gate. Clean-install packaging remains Step 17.
