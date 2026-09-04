# Step 4 boundary classifier contract

Status: preregistered benchmark contract. This file does not select an ASR model and does not authorize new recordings.

## Question

The classifier answers only: does the final 1.0 second contain sustained speech-like activity? It must not reinterpret a single acoustic peak as speech.

## Frozen inputs and features

- Input is mono PCM signed 16-bit WAV at 16 kHz.
- Only the final 16,000 samples are classified.
- Analysis frames are non-overlapping 20 ms (320 samples).
- The legacy comparator passes when the maximum absolute sample is strictly above -45 dBFS.
- An energy-active frame has RMS strictly above -50 dBFS.
- A VAD-active frame is reported by `webrtc-vad = 0.4.0`, 16 kHz, `Aggressive` mode.

In JSON, `primary_pass=true` means speech-like tail activity was detected; a boundary-safe file therefore has `primary_pass=false`.

The primary classifier passes only when both feature families meet both duration conditions:

- energy-active duration is at least 100 ms in total and at least 60 ms consecutively; and
- VAD-active duration is at least 100 ms in total and at least 60 ms consecutively.

These constants are frozen before fixture execution. They must not be relaxed to make an existing human take pass.

## Controls and acceptance

The versioned fixture manifest has named `calibration` and `heldout` splits. Positive controls are constructed by moving known TTS speech into the final second. Negative controls cover silence, an isolated impulse, a click train, stationary noise, breath-like band-limited noise, and a low-frequency handling transient.

Calibration checks implementation and obvious separability only; it is not permission to tune this contract in place. Acceptance requires: both labels occur in each split; the primary classifier makes zero held-out errors; every WAV passes hash, RIFF/PCM, sample-rate, channel, bit-depth, and duration validation; and the report preserves per-sample metrics for both classifiers.

If held-out fails, the result remains diagnostic. Any changed threshold or new spectral feature requires a separately preregistered contract revision and a newly generated held-out split. Existing human takes remain unclassified meanwhile.

## Use on existing takes

Only after the held-out fixture passes may the unchanged primary classifier be applied read-only to take-02 and take-03. A pass means only that this boundary condition is satisfied; it does not make repeated takes independent, representative, or automatically eligible as corpus evidence.
