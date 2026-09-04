# M0 Step 4 local benchmark report — 2026-08-22

Status: incomplete. No production ASR engine or CPU fallback is selected by ADR yet.

## Scope and reproducibility

This is benchmark-only work. It does not add dictation, Notetaker or production sidecar logic.

- Machine: ASUS TUF Gaming A16 FA608PP; AMD Ryzen 9 8940HX (16 cores/32 logical processors), 33,511,448,576 bytes physical RAM.
- OS: Windows 10 Pro 22H2, build 19045.
- Discrete GPU: NVIDIA GeForce RTX 5070 Laptop GPU, 8,151 MiB, driver 596.13. The machine also has an AMD integrated GPU.
- Rust adapter lock: `transcribe-rs=0.3.11`.
- Whisper chain: `whisper-rs=0.16.0` -> `whisper-rs-sys=0.15.0` -> whisper.cpp CPU/Vulkan.
- GigaAM chain: `ort=2.0.0-rc.12` CPU/DirectML through `transcribe-rs`.
- Native pins: Rust 1.97.1 MSVC, LLVM/libclang 22.1.8, `libclang.dll` SHA-256 `51fed10c43c3d31c1fe5bfe76bac60150970961e9b9b23cf014dbfcb5398bbfc`, Vulkan SDK 1.4.357.0, Visual Studio Ninja generator and short Cargo target `.t`.
- Corpus: `wigigadict-sapi-smoke-v1`, ten deterministic RU/EN 16 kHz mono PCM fixtures at exactly 5/15/25/30/60 seconds. This synthetic corpus validates the harness; it is not selection-quality speech.
- Percentiles use nearest-rank. Warm means one unrecorded inference after model load followed by one measured inference in the same worker.

## Exact artifacts

| Candidate artifact | Source revision | Bytes | SHA-256 | Status |
|---|---|---:|---|---|
| whisper.cpp `ggml-large-v3-turbo-q5_0.bin` | `ggerganov/whisper.cpp@5359861c739e955e79d9a303bcbc70fb988958b1` | 574,041,195 | `394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2` | benchmark-only, official exact artifact |
| whisper.cpp `ggml-large-v3-turbo-q8_0.bin` | same revision | 874,188,075 | `317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1` | benchmark-only quality spike; rejected by matched preflight |
| whisper.cpp `ggml-large-v3-q5_0.bin` | same revision | 1,081,140,203 | `d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1` | benchmark-only architecture spike; rejected by matched preflight |
| whisper.cpp `ggml-small-q5_1.bin` | same revision | 190,085,487 | `ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb` | benchmark-only CPU fallback experiment |
| GigaAM-v3 CTC INT8 ONNX | `istupakov/gigaam-v3-onnx@322c3b29492673eb7d0b434bfa9dfb8653e34d02` | 224,721,181 | `ceb61454e2e1a2dec5872cbac1de0fe0a4271d1148f6b26b5bda53ff30a12acd` | benchmark-only exact artifact |
| GigaAM vocab | same revision | 198 | `a9143c30844d3c0bee3e9e927e4084774eb1b9eeaafc473b2c4521e4911a7c07` | required by adapter |

The older local Whisper file with SHA-256 `0dc8...` and cached GigaAM RNNT PyTorch package remain incompatible/unapproved caches and are not reported ready.

## Direct whisper.cpp oracle parity

The Rust adapter was compared with a separately built official `whisper.cpp` v1.8.3 CLI at source commit `2eeeba56e9edd762b4b38467bab96c2517163158`. All 778 vendored binding files matched the official release after deterministic CRLF-to-LF normalization; the normalized tree SHA-256 is `e57f13dc496ed60afabe698aeb047b065e640cf01464522fec6035cba48faab2`. The corrected direct CLI is 57,303,552 bytes with SHA-256 `986ad0eb1026dcf3ef74f38c1678ad691e4b36b5d3e208499c51dcbbc2b341cc`.

Both implementations used the exact large-v3-turbo Q5 model and matched beam size 3, patience -1, four threads, flash attention, no-speech threshold 0.2 and blank/non-speech suppression. On `en-060` and `ru-030`, full text, segment count and every segment text matched exactly. EN timestamps matched exactly; RU boundaries differed by at most 40 ms. The adapter parity criterion therefore passed. Source: `2026-08-22-whisper-oracle-parity.json`.

Both paths also emitted the same hallucinated `Thank you.` segment on `en-060`, ending 26,560 ms after the WAV duration. This is candidate/model behavior, not adapter drift, and remains a reliability blocker. The rejected `istupakov` GigaAM CTC export is not an official GigaAM runtime oracle; a new GigaAM candidate would require separate parity against the official Salute runtime.

## Release matrix results

### Whisper large-v3-turbo Q5 Vulkan

Source: `2026-08-22-whisper-official-vulkan-duration.json`, 20 release records covering both languages and all five durations in cold and warm GPU modes.

| Mode | n | inference p50/p95 | RTF p50/p95 | Peak RAM | Mean WER/CER | Technical-token errors | Truncations | Non-monotonic segments |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| GPU cold | 10 | 381 / 3,884 ms | 0.01683 / 0.06473 | 661,667,840 B | 0.32656 / 0.21772 | 31 / 78 | 2 / 10 | 0 |
| GPU warm | 10 | 271 / 968 ms | 0.01187 / 0.03640 | 661,200,896 B | 0.30436 / 0.21160 | 30 / 78 | 2 / 10 | 0 |

Both 60-second final markers survived. The failures were `ru-025` and `en-015` in both modes. A five-run warm `en-060` repetition was deterministic, had no truncation, p50/p95 693/695 ms and RTF 0.01155/0.01158, but still missed 20/75 synthetic technical tokens.

### Whisper large-v3-turbo Q5 CPU t16

The initial four-thread sanity run was incomplete tuning evidence. An explicit 4/8/16/32 thread screen followed by three warm repetitions at 16/24/32 selected 16 threads for stability: warm p50/p95 was 10,101/10,123 ms, versus 9,699/12,999 ms at 24 threads and 16,408/18,251 ms at 32. Source: `2026-08-22-whisper-large-cpu-thread-sweep.json`.

The full `cpu-t16` report contains 20 RU/EN x duration x cold/warm records. Cold p50/p95 inference was 9,966/35,542 ms with RTF 0.54513/1.99320 and peak RAM 881,008,640 B. Warm p50/p95 was 11,553/35,910 ms with RTF 0.59145/2.29800 and peak RAM 892,370,944 B. Five-second dictation still takes about 9.1-11.5 seconds, while 15-60 second samples run faster than real time; a 60-second sample takes about 35.5 seconds. Quality is close to the GPU candidate on the synthetic corpus, with one cold and two warm truncation detections and no non-monotonic segments. Source: `2026-08-22-whisper-large-cpu-t16-duration.json`.

This makes the same exact-pinned model on CPU the leading functional fallback candidate and avoids a second model package. It is not selected until human speech, clean-machine, power and reliability gates pass.

### Whisper small Q5 CPU experiment

Two five-second release records gave RTF 0.9656–0.9714 and peak RAM 499,638,272 B. Aggregate synthetic WER was 0.71429, 3/4 technical tokens were missed and the RU final marker was absent. This profile is rejected as the CPU fallback candidate.

### GigaAM-v3 CTC INT8

Source: `2026-08-22-gigaam-release-duration-sapi.json`, 40 release records covering RU/EN, all five durations, CPU/DirectML and cold/warm.

| Profile/mode | n | inference p50/p95 | RTF p50/p95 | Peak RAM | Mean WER/CER | Technical-token errors | Truncations |
|---|---:|---:|---:|---:|---:|---:|---:|
| CPU cold | 10 | 925 / 2,816 ms | 0.03700 / 0.04693 | 585,621,504 B | 0.72572 / 0.66827 | 78 / 78 | 7 / 10 |
| CPU warm | 10 | 919 / 2,807 ms | 0.03676 / 0.04678 | 585,957,376 B | 0.72572 / 0.66827 | 78 / 78 | 7 / 10 |
| DirectML cold | 10 | 392 / 529 ms | 0.01540 / 0.08140 | 346,472,448 B | 0.72274 / 0.66960 | 78 / 78 | 7 / 10 |
| DirectML warm | 10 | 77 / 201 ms | 0.00318 / 0.00640 | 346,812,416 B | 0.72274 / 0.66960 | 78 / 78 | 7 / 10 |

English is unsupported in this CTC package: WER is 1.0 and all five final markers are absent. On RU-only samples, CPU/GPU mean WER is 0.45145/0.44548, all 39 technical tokens are missed and two of five final markers are absent (`ru-025`, `ru-060`). The 198-byte Cyrillic vocabulary cannot emit Latin technical identifiers. GigaAM CTC is therefore rejected as both production candidate and CPU fallback despite its speed.

The adapter's DirectML configuration exposes neither device ID nor performance preference. NVIDIA telemetry did not prove RTX residency, so DirectML numbers are not attributed to a specific GPU.

## Crash/restart evidence

The fault hook now loads the actual model/runtime before intentional process exit 86. For both Whisper and GigaAM, CPU and GPU profiles completed baseline -> verified load -> exit 86 -> new worker -> same-sample inference. All four profile pairs produced identical text, model SHA-256 and audio SHA-256. Evidence: `whisper-release-crash-restart.ndjson`, `gigaam-release-crash-restart-verified.ndjson` and transcripts in `logs/`.

This proves adapter worker restart at the benchmark boundary only. Durable lease/checkpoint semantics remain M1 Step 10 work.

## NVIDIA resource evidence

Five separate Whisper Vulkan warm workers processed `en-060` while `nvidia-smi` sampled the exact RTX UUID every 100 ms. Idle was 4.2373 W and 0 MiB. Each process includes model load, warmup and measured inference.

- Peak incremental VRAM: 926,941,184 bytes (884 MiB) in every run.
- Average incremental power: 18.9881–31.4651 W; nearest-rank p50/p95 27.9744/31.4651 W.
- Incremental process energy: `1.38058e-5`–`2.40991e-5` kWh; nearest-rank p50/p95 `2.00315e-5`/`2.40991e-5` kWh.
- Process elapsed time: 2,551–2,757 ms; 17 telemetry samples per run.

Source: `2026-08-22-whisper-vulkan-nvidia-telemetry.json`. CPU wall power is not available from a trustworthy built-in sensor on this machine. DirectML energy/VRAM remains unreported because the adapter cannot select or identify its device.

## Battery whole-system resource evidence

Three separate Whisper CPU `cpu-t16` warm workers processed `en-060` on battery under the `Turbo` power scheme. Idle whole-system discharge averaged 25.1077 W. The benchmark process measurement includes model load, one unrecorded warmup inference and one measured inference.

- Measured inference p50/p95: 217,217/218,319 ms; RTF p50/p95: 3.62028/3.63865.
- Whole-process elapsed p50/p95: 440,954.071/445,091.629 ms.
- Whole-system battery discharge p50/p95: 55.5712/57.1596 W.
- Idle-subtracted discharge p50/p95: 30.3884/31.9744 W.
- Whole-system energy p50/p95: 0.0068538352/0.0069844324 kWh; idle-subtracted energy p50/p95: 0.0037571229/0.003916458 kWh.
- Peak worker RAM: 874,463,232 B.

All three evidence rows used the same runtime, 16 threads and immutable model/audio hashes. Text and segments were identical and timestamps were monotonic. The known hallucinated `Thank you.` suffix also reproduced identically, ending at 86,520 ms for a 60,000 ms WAV.

This is supporting whole-system power evidence, not a passed CPU-package or AC-wall-power gate. The machine's `BatteryStaticData` capability query fails, so relative battery units cannot be independently ruled out. Two brief read-only status snapshots were taken during long silent intervals and remain measurement noise in the logs. The profile is substantially slower than real time on battery and fails the battery performance gate. Sources: `2026-08-22-whisper-large-cpu-t16-battery.json` and `2026-08-22-whisper-large-cpu-t16-battery-quality.json`.

## Human speech evidence

The first private take is retained as rejected diagnostic evidence: its ten WAV files were valid PCM but effectively silent (maximum observed peak -90.3 dB), and the original recorder also captured ten empty `Read-Host` results in provenance. The benchmark-only recorder now requires PowerShell 7.4, discards prompt output and fails closed after a five-second microphone preflight when the peak is below -50 dB.

`speaker-a/take-02` passed that preflight at mean -21.5 dB and peak -2.6 dB. All ten private WAVs are 16 kHz mono signed 16-bit PCM with matching hashes; their sample peaks range from -3.3 to -1.5 dB. Raw audio and transcripts remain ignored. Aggregate capture evidence is versioned in `2026-08-22-human-corpus-speaker-a-take02-assessment.json`.

The exact Vulkan-capable worker is 57,011,200 bytes with SHA-256 `cb05d28cf8baa67b3a593747dc5efad6bb03fcafe33da3810249b3cb6bb84177`. It was run once with the RTX Vulkan backend and once with GPU disabled plus 16 CPU threads:

| Profile/mode | n | inference p50/p95 | RTF p50/p95 | Peak RAM | Mean WER/CER | Technical-token errors | Truncations | Non-monotonic segments |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Vulkan cold | 10 | 351 / 941 ms | 0.01569 / 0.04995 | 661,032,960 B | 0.46946 / 0.25309 | 38 / 78 | 6 / 10 | 0 |
| Vulkan warm | 10 | 259 / 868 ms | 0.01287 / 0.03089 | 660,938,752 B | 0.45673 / 0.26047 | 39 / 78 | 6 / 10 | 0 |
| Shared worker CPU t16 cold | 10 | 17,103 / 58,959 ms | 0.75340 / 3.43089 | 927,817,728 B | 0.46395 / 0.25247 | 35 / 78 | 7 / 10 | 0 |
| Shared worker CPU t16 warm | 10 | 17,634 / 53,449 ms | 0.88542 / 3.39218 | 931,516,416 B | 0.45847 / 0.25968 | 36 / 78 | 6 / 10 | 0 |

CPU and Vulkan produced exact full text and exact segment text in 9 of 20 paired runs; 19 of 20 segment counts matched, with a maximum boundary delta of 7,000 ms. Both satisfy the canonical output shape and monotonicity requirement, but backend-specific decoding is not bit-identical and must retain runtime provenance.

The two five-second captures with active speech in their final second (`ru-005`, `en-005`) also missed their final markers. Those misses cannot be attributed solely to the model because the exact-duration recorder cut while speech was still active. The other misses remain model-quality evidence. One accepted take from one speaker is not a selection-quality distribution.

A fresh standalone `whisper-cpu` build regressed to 109,383 ms on one five-second human sample and timed out at 90 seconds in controlled retries. It is rejected as current evidence. The same exact Vulkan-capable worker, with GPU disabled and `--threads 16`, completed controlled five-second retries in 9,904-11,206 ms and the full human matrix above. The cause of the standalone-build regression and clean-machine portability of the shared worker remain unresolved.

### Boundary-safe human take 04

An exact-duration third take passed PCM and signal checks but retained active speech in the final second of 7/10 files. It is preserved as boundary-confounded evidence. The recorder now supports explicit tail padding while storing prompt and capture durations separately. `speaker-a/take-04` used 3,000 ms padding; all ten last seconds were quiet, all hashes and PCM invariants passed, and no capture-boundary qualification is needed.

| Profile/mode | n | inference p50/p95 | RTF p50/p95 | Peak RAM | Mean WER/CER | Technical-token errors | Truncations | Non-monotonic segments |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Vulkan cold | 10 | 415 / 1,057 ms | 0.01678 / 0.03093 | 661,676,032 B | 0.57659 / 0.30292 | 33 / 78 | 9 / 10 | 0 |
| Vulkan warm | 10 | 265 / 823 ms | 0.01133 / 0.01916 | 661,086,208 B | 0.52497 / 0.30751 | 33 / 78 | 9 / 10 | 0 |
| Shared worker CPU t16 cold | 10 | 12,849 / 56,782 ms | 0.67254 / 1.62680 | 953,143,296 B | 0.56155 / 0.30741 | 33 / 78 | 9 / 10 | 0 |
| Shared worker CPU t16 warm | 10 | 17,714 / 54,117 ms | 0.97181 / 2.20789 | 954,273,792 B | 0.51175 / 0.29218 | 30 / 78 | 9 / 10 | 0 |

Only `ru-030` retained its final marker in both modes and both backends: 2/20 measured rows per backend. CPU and Vulkan exact full text matched in 6/20 pairs, segment count in 17/20 and maximum boundary delta was 1,120 ms. All intervals remained monotonic. With capture clipping removed, the marker and WER results are model/profile evidence and fail the current human quality gate.

### Large-v3-turbo Q8 matched quality preflight

The official Q8_0 artifact from the same immutable revision was evaluated on the exact same boundary-safe `take-04` RU/EN 5/30/60-second samples in cold and warm Vulkan modes. The exact accepted worker and decode settings were unchanged; only model quantization changed. The 12-record report is `2026-08-22-whisper-large-turbo-q8-vulkan-human-speaker-a-take04-preflight.json`, SHA-256 `90187c5aa366545a249ac3b25433a750597a850657b402ec1c2d6028a7a17037`.

| Candidate/mode | n | inference p50/p95 | RTF p50/p95 | Peak RAM | Mean WER/CER | Technical-token errors | Final markers | Non-monotonic segments |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Q5 cold matched subset | 6 | 517 / 1,057 ms | 0.01678 / 0.03093 | 661,676,032 B | 0.64538 / 0.31858 | 23 / 54 | 1 / 6 | 0 |
| Q8 cold | 6 | 775 / 3,778 ms | 0.03006 / 0.47314 | 976,404,480 B | 0.64877 / 0.32860 | 24 / 54 | 0 / 6 | 0 |
| Q5 warm matched subset | 6 | 367 / 823 ms | 0.01307 / 0.01916 | 661,086,208 B | 0.57175 / 0.32473 | 23 / 54 | 1 / 6 | 0 |
| Q8 warm | 6 | 477 / 897 ms | 0.01446 / 0.01991 | 964,550,656 B | 0.66164 / 0.32601 | 24 / 54 | 1 / 6 | 0 |

Across all 12 matched rows, Q8 recovered 1/12 markers versus Q5's 2/12, mean WER regressed from 0.60857 to 0.65521, and mean CER regressed from 0.32166 to 0.32730. The full Q8 matrix was therefore intentionally skipped under the predeclared gate. This rejects Q8_0 as the next candidate; it does not select Q5_0 or complete Step 4.

### Non-turbo large-v3 Q5 matched quality preflight

The official non-turbo large-v3 Q5_0 artifact from the same immutable revision was evaluated on the same boundary-safe `take-04` RU/EN 5/30/60-second cold/warm subset. Quantization, worker, adapter, decode settings and Vulkan profile were held constant; the text decoder changed from turbo's 4 layers to large-v3's 32 layers. The 12-record report is `2026-08-22-whisper-large-v3-q5-vulkan-human-speaker-a-take04-preflight.json`, SHA-256 `45e13f7c64dbb24a2053e35ffe27e6993c7130459a87ba2b59504ec6afcd22b8`.

| Candidate/mode | n | inference p50/p95 | RTF p50/p95 | Peak RAM | Mean WER/CER | Technical-token errors | Final markers | Non-monotonic segments |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Turbo Q5 cold matched subset | 6 | 517 / 1,057 ms | 0.01678 / 0.03093 | 661,676,032 B | 0.64538 / 0.31858 | 23 / 54 | 1 / 6 | 0 |
| Large-v3 Q5 cold | 6 | 1,082 / 2,571 ms | 0.04082 / 0.06162 | 1,162,854,400 B | 0.55426 / 0.30872 | 21 / 54 | 1 / 6 | 0 |
| Turbo Q5 warm matched subset | 6 | 367 / 823 ms | 0.01307 / 0.01916 | 661,086,208 B | 0.57175 / 0.32473 | 23 / 54 | 1 / 6 | 0 |
| Large-v3 Q5 warm | 6 | 1,085 / 2,185 ms | 0.03469 / 0.05147 | 1,162,743,808 B | 0.59933 / 0.33482 | 26 / 54 | 1 / 6 | 0 |

Across all 12 matched rows, large-v3 Q5 kept marker recovery unchanged at 2/12, improved mean WER from 0.60857 to 0.57680, and slightly regressed mean CER from 0.32166 to 0.32177. Technical-token errors rose from 46/108 to 47/108, peak RAM increased by about 501 MB, and p95 RTF remained below real time but roughly doubled. Because marker and CER gates did not improve, the full matrix was intentionally skipped. This candidate is not selected and Step 4 remains incomplete.

## Benchmark package footprint

These are lower bounds for benchmark workers, not installer-size promises; system MSVC/Vulkan/D3D runtimes and future manifests/licenses are excluded.

| Chain | Worker | Extra runtime | Model/vocab | Lower bound |
|---|---:|---:|---:|---:|
| Whisper Vulkan large Q5 | 57,008,128 B | system Vulkan loader | 574,041,195 B | 631,049,323 B |
| GigaAM DirectML INT8 | 21,088,768 B | DirectML.dll 18,527,776 B | 224,721,379 B | 264,337,923 B |

## Decision status

- GigaAM-v3 CTC INT8: rejected for v1 mixed RU/EN technical dictation and rejected as CPU fallback.
- Whisper small Q5 CPU: rejected as CPU fallback on current evidence.
- Whisper large-v3-turbo Q8 Vulkan: rejected by the matched boundary-safe quality preflight; no full matrix was run.
- Whisper large-v3 Q5 Vulkan: rejected by the matched architecture preflight; WER improved, but marker/CER gates did not and resource use increased.
- Whisper large-v3-turbo Q5 Vulkan: fastest local candidate, but the boundary-safe human take fails marker/WER quality; not selected and not production-approved.
- CPU fallback: unresolved; the same Vulkan-capable worker and model with GPU disabled is the leading functional path, but human p95 RTF is about 3.4, measured battery RTF is 3.62, and the standalone CPU build regressed.
- Selection ADR: intentionally not created until remaining gates pass.

Step 4 remains unchecked because the first boundary-safe human take fails the marker/WER quality gate and still represents one speaker, a clean Windows machine run is absent, trustworthy CPU-package/AC-wall power is unavailable, the CPU fallback performance/rebuild story is unresolved, the out-of-bounds Whisper hallucination is unresolved, and production supply-chain policy still rejects `Unlicense` (Whisper bindings) plus `CDLA-Permissive-2.0` (ORT build tooling). Direct pinned-runtime Whisper oracle parity now passes. No license exception was added.

## Required next evidence

1. Add at least two more boundary-safe independent recordings per prompt, including another voice if available, and rerun WER/CER/token/truncation gates. Keep exact-duration tail clipping separate from model truncation.
2. Run the exact locked harness and artifacts on a clean supported Windows 10 machine without Python/Hugging Face caches or manual DLL repair.
3. Treat the Whisper adapter oracle gate as passed. If a new GigaAM candidate replaces the rejected third-party CTC export, compare it with the exact official Salute runtime before selection.
4. Resolve the standalone CPU-build regression, validate the exact shared-worker fallback on clean Windows, then benchmark a battery-appropriate profile. Current human p95 and battery `cpu-t16` evidence fail real-time operation. Do not relabel the failed GigaAM CTC or Whisper small profiles.
5. Treat BatteryStatus as supporting whole-system evidence only. Obtain a trustworthy CPU package sensor or external wall meter if the strict watts gate remains required, and resolve DirectML device attribution if that chain remains under consideration.
6. Resolve production license/SBOM policy before any benchmark dependency enters the sidecar.
