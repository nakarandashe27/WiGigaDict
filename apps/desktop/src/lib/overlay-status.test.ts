import { describe, expect, it } from "vitest";
import {
  isTerminalOverlayPhase,
  overlayCompactLabel,
  overlayCopy,
  overlayWaveformScale,
  type OverlayPhase,
} from "./overlay-status";

describe("overlay presentation policy", () => {
  it("reports the safety limit as an ongoing recognition, not as a failure", () => {
    const copy = overlayCopy({
      phase: "processing",
      sessionId: "s1",
      reason: "pcm_size_limit",
    });
    expect(copy.title).toContain("предел");
    expect(isTerminalOverlayPhase("processing")).toBe(false);
  });

  it("reports an unavailable recogniser as a terminal outcome, not as processing", () => {
    const copy = overlayCopy({
      phase: "error",
      sessionId: "s1",
      reason: "asr_unavailable",
    });
    expect(copy.title).toBe("Распознавание недоступно");
    expect(copy.detail).toContain("восстановление");
    expect(isTerminalOverlayPhase("error")).toBe(true);
    expect(isTerminalOverlayPhase("processing")).toBe(false);
  });

  it("claims insertion only for evidence-backed delivered", () => {
    expect(
      overlayCopy({ phase: "delivered", sessionId: "s1", reason: null }).title,
    ).toBe("Текст вставлен");
    expect(
      overlayCopy({
        phase: "uncertain",
        sessionId: "s1",
        reason: "delivery_unconfirmed",
      }).title,
    ).toContain("не подтверждена");
  });

  it("names the uncertain sub-cases apart for the owner", () => {
    expect(
      overlayCopy({
        phase: "uncertain",
        sessionId: "s1",
        reason: "delivery_transport_only",
      }).title,
    ).toBe("Текст вставлен (без подтверждения)");
    expect(
      overlayCopy({
        phase: "uncertain",
        sessionId: "s1",
        reason: "empty_transcript",
      }).title,
    ).toContain("пустой");
    expect(
      overlayCopy({
        phase: "processing",
        sessionId: "s1",
        reason: "processing_cpu_fallback",
      }).title,
    ).toContain("CPU");
  });

  it("maps content-free recovery reasons to one next step", () => {
    const copy = overlayCopy({
      phase: "error",
      sessionId: "s1",
      reason: "audio_device_lost",
    });
    expect(copy.title).toContain("Микрофон");
    expect(copy.detail).toContain("recovery");
  });

  it("auto-hides only terminal presentation phases", () => {
    const active: OverlayPhase[] = ["preparing", "recording", "processing"];
    const terminal: OverlayPhase[] = ["delivered", "uncertain", "error"];
    expect(active.every((phase) => !isTerminalOverlayPhase(phase))).toBe(true);
    expect(terminal.every(isTerminalOverlayPhase)).toBe(true);
  });

  it("uses compact content-free labels in the floating pill", () => {
    expect(
      overlayCompactLabel({
        phase: "processing",
        sessionId: "s1",
        reason: null,
      }),
    ).toBe("Обработка");
    expect(
      overlayCompactLabel({
        phase: "error",
        sessionId: "s1",
        reason: "private backend detail",
      }),
    ).toBe("Проверьте");
  });

  it("keeps quiet speech visible while preserving waveform dynamics", () => {
    const quiet = overlayWaveformScale(0.04, 1);
    const loud = overlayWaveformScale(0.9, 1);
    expect(quiet).toBeGreaterThanOrEqual(0.28);
    expect(loud).toBeGreaterThan(quiet);
    expect(loud).toBeLessThanOrEqual(1);
  });
});
