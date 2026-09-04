import { describe, expect, it } from "vitest";
import {
  canDelete,
  canRetry,
  recoveryLabel,
  type RecoveryEntry,
} from "./recovery";

function entry(overrides: Partial<RecoveryEntry> = {}): RecoveryEntry {
  return {
    sessionId: "session-1",
    pipelineState: "recovery",
    stateVersion: 3,
    status: "uncertain",
    recoveryRequired: true,
    raw: null,
    cleaned: null,
    selected: {
      transcriptId: "raw-1",
      sessionId: "session-1",
      kind: "raw",
      content: "recoverable text",
      contentHash: "a".repeat(64),
      createdAt: 1,
    },
    operations: [],
    startedAt: 1,
    updatedAt: 2,
    deliveredAt: null,
    resolvedAt: null,
    pinned: false,
    retentionExpiresAt: null,
    lastErrorCode: "delivery_uncertain",
    ...overrides,
  };
}

describe("recovery view policy", () => {
  it("offers retry only for unresolved entries with a transcript", () => {
    expect(canRetry(entry())).toBe(true);
    expect(canRetry(entry({ recoveryRequired: false }))).toBe(false);
    expect(canRetry(entry({ selected: null }))).toBe(false);
  });

  it("does not offer delete while a pipeline still owns the session", () => {
    expect(canDelete(entry())).toBe(true);
    expect(canDelete(entry({ pipelineState: "processing" }))).toBe(false);
    expect(canDelete(entry({ pipelineState: "delivering" }))).toBe(false);
  });

  it("keeps copied distinct from resolved", () => {
    expect(recoveryLabel("copied")).toContain("не решено");
    expect(recoveryLabel("resolved")).toBe("Решено");
  });
});
