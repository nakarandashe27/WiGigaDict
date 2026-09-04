# M0 Step 4 boundary classifier experiment

Date: 2026-08-23

## Outcome

The preregistered primary classifier passed the labeled held-out fixture with zero false positives and zero false negatives. The legacy `peak > -45 dBFS` comparator produced false positives on controls containing no speech, so peak alone is not a valid proxy for speech in the tail.

No microphone, playback, listening, or ASR matrix was used.

## Frozen method

- Final 1.0 second only; mono PCM16 at 16 kHz; 20 ms frames.
- Legacy comparator: maximum absolute sample strictly above -45 dBFS.
- Primary speech classifier: WebRTC VAD 0.4.0 in Aggressive mode AND RMS above -50 dBFS, each at least 100 ms total and 60 ms consecutively.
- Thresholds were recorded in `tests/asr-benchmark/boundary-contract.md` before fixture execution and were not changed after results.
- WAV identity and provenance are bound by SHA-256 in the ignored fixture/diagnostic manifests and preserved in sanitized JSON reports.

## Controls

The fixture contains 12 deterministic controls across calibration and held-out splits. Positive controls move existing RU/EN TTS speech into the final 0.7 seconds at two gains. Negative controls cover silence, an isolated -41.94 dBFS impulse, a click train, stationary white noise, band-limited pink noise, and low-frequency handling transients.

| Split | Classifier | FP | FN |
|---|---:|---:|---:|
| Calibration | Legacy peak | 3 | 0 |
| Calibration | Primary | 0 | 0 |
| Held-out | Legacy peak | 2 | 0 |
| Held-out | Primary | 0 | 0 |

Legacy false positives were the calibration impulse, white noise and handling transient, plus the held-out click train and handling transient. This directly demonstrates that an acoustic peak above -45 dBFS does not establish speech.

The machine-readable fixture result is `2026-08-23-boundary-classifier.json`; its fixture manifest SHA-256 is `f4f91d2b47aee98d506a246428ef4a31e0bd974c2a7caff6c6de7ab9b40a294f`.

## Existing speaker-b takes (diagnostic inference)

The unchanged classifier was applied read-only to all 20 WAVs in take-02 and take-03. The diagnostic manifest used unknown labels, so the report makes no correctness claim and does not use human files as tuning data.

| Take | Files | Legacy peak detects activity | Primary detects sustained speech-like tail |
|---|---:|---:|---:|
| take-02 | 10 | 2 | 0 |
| take-03 | 10 | 1 | 0 |

The three legacy-active tails were:

- `take-02/en-015`: peak -42.01 dBFS; 0 ms energy-active; 0 ms VAD-active.
- `take-02/ru-005`: peak -42.42 dBFS; 20 ms energy-active; 80 ms VAD-active; below both frozen 100 ms total requirements.
- `take-03/ru-025`: peak -43.27 dBFS; 0 ms energy-active; 0 ms VAD-active.

Therefore take-02 and take-03 both satisfy the frozen boundary condition (`primary_pass=false` for every file). This does not restore independence between repeated recordings.

## Evidence decision

- take-02 is the earliest repeated speaker-b take that satisfies the frozen validated boundary classifier and may be used as the single speaker-b evidence take, subject to its already-recorded hash/PCM/duration/provenance checks.
- take-03 is boundary-safe but remains diagnostic because it is a later repeated recording of the same corpus/session.
- take-01 remains diagnostic because it preceded the frozen validated classifier and showed 7/10 legacy-active tails.
- take-04 is empty and is not evidence.
- No further user recording is needed for this boundary question.

## Limits

The controls validate the named failure modes, not universal speech detection. TTS positives and synthetic noise are not a substitute for a large annotated human VAD benchmark. Spectral rules are not added because the held-out fixture and the actual borderline tails were resolved by duration plus VAD. A future failure on a named acoustic class would require a new preregistered experiment and fresh held-out controls, not an in-place threshold change.
