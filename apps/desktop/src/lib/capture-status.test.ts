import { audioGapWarning, type CaptureStatus } from "./capture-status";

function status(overrides: Partial<CaptureStatus> = {}): CaptureStatus {
  return {
    phase: "recording",
    sessionId: "s-1",
    reason: null,
    deviceHealthy: true,
    durablePcmBytes: 4096,
    audioGaps: 0,
    ...overrides,
  };
}

describe("dropped audio", () => {
  it("says nothing while the device keeps up", () => {
    expect(audioGapWarning(status())).toBeNull();
  });

  it("warns that words may be missing once samples were dropped", () => {
    const one = audioGapWarning(status({ audioGaps: 1 }));
    expect(one).toContain("один раз");
    expect(one).toContain("может не хватать слов");
    expect(audioGapWarning(status({ audioGaps: 3 }))).toContain("3 раза");
  });
});

import { describe, expect, it } from "vitest";
import { canCancelCapture, captureLabel } from "./capture-status";

describe("capture status presentation", () => {
  it("keeps active capture visible and cancellable", () => {
    expect(captureLabel("preparing")).toContain("Подготовка");
    expect(captureLabel("recording")).toBe("Запись");
    expect(canCancelCapture("preparing")).toBe(true);
    expect(canCancelCapture("recording")).toBe(true);
  });

  it("does not issue late cancellation after finalization begins", () => {
    expect(canCancelCapture("finalizing")).toBe(false);
    expect(canCancelCapture("idle")).toBe(false);
    expect(canCancelCapture("recovery")).toBe(false);
    expect(canCancelCapture("unavailable")).toBe(false);
    expect(captureLabel("recovery")).toContain("сохранена");
  });
});
