import { describe, expect, it } from "vitest";
import {
  DIAGNOSTIC_EXPORT_CONFIRMATION,
  formatDiagnosticBytes,
  validateDiagnosticDestination,
} from "./diagnostics";

describe("diagnostic bundle boundary", () => {
  it("requires an absolute Windows destination with the versioned extension", () => {
    expect(validateDiagnosticDestination("support.json")).toContain("полный");
    expect(validateDiagnosticDestination("C:\\Temp\\support.json")).toContain(
      ".wigigadiag.json",
    );
    expect(
      validateDiagnosticDestination("C:\\Temp\\support.wigigadiag.json"),
    ).toBeNull();
  });

  it("keeps confirmation stable and formats bounded sizes", () => {
    expect(DIAGNOSTIC_EXPORT_CONFIRMATION).toBe(
      "export_content_free_diagnostics",
    );
    expect(formatDiagnosticBytes(512)).toBe("512 Б");
    expect(formatDiagnosticBytes(2048)).toBe("2.0 КиБ");
    expect(formatDiagnosticBytes(-1)).toBe("—");
  });
});
