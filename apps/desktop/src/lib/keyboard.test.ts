import { describe, expect, it } from "vitest";
import { nextDialogControl } from "./keyboard";

describe("keyboard-only dialog policy", () => {
  it("keeps forward and reverse Tab inside the two dialog actions", () => {
    expect(nextDialogControl("cancel", false)).toBe("confirm");
    expect(nextDialogControl("confirm", true)).toBe("cancel");
  });

  it("allows native movement between internal controls", () => {
    expect(nextDialogControl("confirm", false)).toBeNull();
    expect(nextDialogControl("cancel", true)).toBeNull();
  });
});
