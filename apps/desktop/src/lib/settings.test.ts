import { describe, expect, it } from "vitest";
import {
  configurationUpdate,
  runtimeLabel,
  validateConfiguration,
  type AppConfiguration,
} from "./settings";

const configuration: AppConfiguration = {
  configVersion: 3,
  hotkeyBinding: "F8",
  microphoneDeviceId: null,
  activeRuntimeProfileId: null,
  activeCleanupProfileId: null,
  startupEnabled: false,
  warmupEnabled: false,
  diagnosticMode: false,
  archiveDirectory: String.raw`C:\Users\Owner\Documents\WiGigaDict`,
};

describe("settings policy", () => {
  it("preserves the optimistic snapshot version", () => {
    expect(configurationUpdate(configuration).expectedConfigVersion).toBe(3);
  });

  it("allows a function key and requires a runtime for warm-up", () => {
    expect(
      validateConfiguration({
        ...configurationUpdate(configuration),
        hotkeyBinding: "F8",
      }),
    ).toBeNull();
    expect(
      validateConfiguration({
        ...configurationUpdate(configuration),
        hotkeyBinding: "Space",
      }),
    ).toContain("F1–F12");
    expect(
      validateConfiguration({
        ...configurationUpdate(configuration),
        warmupEnabled: true,
      }),
    ).toContain("модель");
  });

  it("labels local runtime by model and device", () => {
    expect(
      runtimeLabel({
        id: "gpu",
        modelName: "Whisper large-v3-turbo",
        modelVersion: "Q5",
        deviceKind: "vulkan",
        healthState: "healthy",
        available: true,
      }),
    ).toContain("GPU");
  });

  it("requires an absolute local archive directory", () => {
    expect(
      validateConfiguration({
        ...configurationUpdate(configuration),
        archiveDirectory: "relative-folder",
      }),
    ).toContain("локальной папке");
    expect(
      validateConfiguration({
        ...configurationUpdate(configuration),
        archiveDirectory: String.raw`D:\Dictation archive`,
      }),
    ).toBeNull();
  });
});
