import { describe, expect, it } from "vitest";

import { APP_VERSION, PROTOCOL_VERSION } from "./version";

describe("development version identity", () => {
  it("keeps the shell and shared fixture protocol pinned", () => {
    expect(APP_VERSION).toBe("0.0.1-dev");
    expect(PROTOCOL_VERSION).toBe("0.2.0");
  });
});
