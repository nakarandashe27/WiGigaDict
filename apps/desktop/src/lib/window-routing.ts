export type WindowLabel = "main" | "overlay";

export interface WindowRoutingInput {
  development: boolean;
  previewWindow: string | null;
  currentWindowLabel: string | null;
}

export function resolveWindowLabel({
  development,
  previewWindow,
  currentWindowLabel,
}: WindowRoutingInput): WindowLabel {
  if (development && previewWindow === "overlay") return "overlay";
  return currentWindowLabel === "overlay" ? "overlay" : "main";
}
