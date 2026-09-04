import { describe, expect, it } from "vitest";

import { resolveWindowLabel } from "./window-routing";

describe("resolveWindowLabel", () => {
  it("renders the real Tauri overlay during development without a query string", () => {
    expect(
      resolveWindowLabel({
        development: true,
        previewWindow: null,
        currentWindowLabel: "overlay",
      }),
    ).toBe("overlay");
  });

  it("keeps the browser-only overlay preview", () => {
    expect(
      resolveWindowLabel({
        development: true,
        previewWindow: "overlay",
        currentWindowLabel: null,
      }),
    ).toBe("overlay");
  });

  it("defaults unknown and production main windows to the main application", () => {
    expect(
      resolveWindowLabel({
        development: true,
        previewWindow: null,
        currentWindowLabel: null,
      }),
    ).toBe("main");
    expect(
      resolveWindowLabel({
        development: false,
        previewWindow: "overlay",
        currentWindowLabel: "main",
      }),
    ).toBe("main");
  });
});
