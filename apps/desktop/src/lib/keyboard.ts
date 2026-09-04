export type DialogControl = "confirm" | "cancel";

export function nextDialogControl(
  active: DialogControl,
  backwards: boolean,
): DialogControl | null {
  if (backwards && active === "confirm") return "cancel";
  if (!backwards && active === "cancel") return "confirm";
  return null;
}
