# WiGigaDict recording overlay design QA

- Source visual truth: `C:\Local WhisperGigaAM Desktop\artifacts\reference-recording-overlay.png`
- Implementation: `apps/desktop/src/Overlay.tsx` and `apps/desktop/src/styles.css`
- Earlier before screenshot: `C:\Local WhisperGigaAM Desktop\artifacts\recording-overlay-before-fidelity-pass.png`.
- User-reported broken recording screenshot: `C:\Local WhisperGigaAM Desktop\artifacts\broken-recording-overlay.png`.
- User-reported broken processing screenshot: `C:\Local WhisperGigaAM Desktop\artifacts\broken-processing-overlay.png`.
- After screenshot: recording captured on 2026-09-03 through Playwright CLI at the exact 184 × 44 CSS viewport after the in-app Browser kernel failed during setup. The screenshot is a temporary QA artifact outside the repository.
- Source pixels: 391 × 456.
- Source target region: bottom recording pill, approximately 101 × 36 px inside the source image.
- Implementation CSS size: 100 × 34 px recording pill and 148 × 34 px status pill, centered in one phase-invariant 184 × 44 logical px native viewport.
- Implementation pixels and device scale factor: 184 × 44 CSS pixels at screenshot scale `css`; recording card measured from the rendered result after the fidelity correction.
- Density normalization: not needed for the Playwright capture because it was emitted at CSS scale and the target viewport is expressed in logical CSS pixels.
- State: `recording`.

## Full-view comparison evidence

Partially complete. The source image and the fresh browser-rendered recording state were compared in one review. The first fresh capture exposed a 148 px recording capsule instead of the specified compact 100 px state; `.overlay-recording` now restores the 100 px width while the native WebView remains 184 × 44. The second capture confirms the corrected compact capsule without clipping or red-corner artifacts.

The implemented composition follows the source structure: a compact dark capsule, restrained light outline, circular cancel and finish controls, and a centered free-bar waveform. The waveform now uses one Phosphor `Waveform` icon with `bold` weight and responds to a content-free live microphone-level event. Every phase keeps the same native WebView extent; only the centered card content changes.

## Focused region comparison evidence

The recording region is now verified at the target scale. A fresh processing screenshot is still missing: a later read-only capture request was rejected by the external approval service, so the transition cannot yet be marked fully certified.

## Findings

- [P2] Processing-state visual fidelity is not yet certified
  - Location: recording overlay.
  - Evidence: recording has fresh browser evidence; processing still has only the earlier broken physical screenshot and source/CSS review.
  - Impact: the corrected processing label, sparkle alignment and final optical spacing are not proven by fresh after-evidence.
  - Fix: capture `http://127.0.0.1:1420/?window=overlay&phase=processing` at a 184 × 44 CSS viewport when the approval service is available.

- [P1] Physical state transition still needs after-evidence
  - Location: recording to processing transition.
  - Evidence: the supplied broken screenshots show clipped/transient rendering while the previous implementation resized the transparent native WebView between 100 × 36 and 184 × 44. That resize path has been removed, but no post-fix physical screenshot is available.
  - Impact: the exact absence of clipping, red corner artifacts, and perceived lag cannot yet be certified.
  - Fix: capture the revised recording state and the subsequent processing state from one physical F8 toggle cycle.

## Required fidelity surfaces

- Fonts and typography: the recording pill intentionally contains no visible text, matching the source target region.
- Spacing and layout rhythm: source target is approximately 101 × 36 px; implementation recording card is 100 × 36 px with 20 px circular endcaps and a 34 × 14 px center meter; visual verification is blocked.
- Colors and visual tokens: solid near-black fill, a one-pixel cool-gray outline, muted gray endcaps, and white foreground icons follow the source; visual verification is blocked.
- Image quality and asset fidelity: the target region contains no raster product asset. Controls and waveform use `@phosphor-icons/react`; no handcrafted SVG, emoji, placeholder, gradient, or CSS-drawn waveform is used.
- Copy and content: no visible copy appears in the recording state; the accessible status label remains content-free.

## Comparison history

- Pass 1: the source target was measured and the recording HUD was rebuilt to match its component dimensions and structure.
- Pass 2: a user-provided implementation screenshot exposed a filled rectangular waveform, oversized endcaps, and a visually heavy warm outline. The recording state was corrected to use a pair of free-bar `Waveform` icons, 20 px endcaps, a one-pixel cool outline, a solid near-black fill, and valid WebView2-compatible live level scaling. The full frontend quality gate passed after these changes.
- Pass 3: an automated after-capture was attempted through the required in-app Browser runtime; the Node kernel exited before rendering. Final paired comparison therefore remains blocked pending a fresh physical screenshot.
- Pass 4: the user supplied recording and processing screenshots showing a clipped, visually unstable transition. The phase-dependent native resize was identified as the mechanical cause and removed. The overlay now keeps one stable transparent WebView extent, renders smaller centered cards, uses a single waveform icon, and avoids blur plus animated box-shadow work. Rust tests (45), frontend unit tests (20), integration tests (3), typecheck, lint, formatting, and production build pass. Automated after-capture remains blocked by the Windows sandbox helper.
- Pass 5: Playwright CLI rendered the overlay at the exact 184 × 44 viewport. The recording capsule was still 148 px wide because the compact-state width override had been lost; the fresh screenshot made the mismatch visible. `.overlay-recording` now restores the 100 px card width and adds an interruptible width transition inside the unchanged native viewport. A second recording screenshot confirms the compact state without clipping. Processing after-evidence remains pending because the capture request was rejected by the approval service.

## Implementation checklist

- Capture the revised recording and processing pills through one physical F8 toggle cycle; recording already has browser after-evidence.
- Compare the fresh processing screenshot with its source target region in one visual input.
- Correct any remaining P1/P2 outline, spacing, shadow, opacity, or icon-scale differences.

## Follow-up polish

- Tune the endcap opacity and waveform optical weight only if the real paired capture still differs from the source.

final result: recording passed; physical recording → processing transition pending
