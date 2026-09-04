export const DIAGNOSTIC_EXPORT_CONFIRMATION = "export_content_free_diagnostics";

export type DiagnosticView = {
  expandedEventsEnabled: boolean;
  traceSchemaVersion: number;
  retentionDays: number;
  maximumBytes: number;
  fileCount: number;
  storedBytes: number;
  eventCount: number;
};

export type DiagnosticBundlePreview = {
  previewId: string;
  bundleSchemaVersion: number;
  eventCount: number;
  sourceFileCount: number;
  byteCount: number;
  firstOccurredAt: number | null;
  lastOccurredAt: number | null;
  redactionCount: number;
  excludedByDefault: string[];
};

export type DiagnosticExportReceipt = {
  fileName: string;
  byteCount: number;
  eventCount: number;
};

export function formatDiagnosticBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "—";
  if (value < 1024) return value + " Б";
  if (value < 1024 * 1024) return (value / 1024).toFixed(1) + " КиБ";
  return (value / (1024 * 1024)).toFixed(1) + " МиБ";
}

export function validateDiagnosticDestination(value: string): string | null {
  if (!value) return "Укажите полный путь для сохранения файла.";
  if (value.length > 1024 || value.includes("\0"))
    return "Путь слишком длинный или содержит недопустимый символ.";
  if (!/^[a-zA-Z]:\\/.test(value) && !/^\\\\[^\\]+\\[^\\]+\\/.test(value))
    return "Нужен полный Windows-путь, например C:\\Users\\…\\support.wigigadiag.json.";
  if (!value.endsWith(".wigigadiag.json"))
    return "Имя файла должно оканчиваться на .wigigadiag.json.";
  return null;
}
