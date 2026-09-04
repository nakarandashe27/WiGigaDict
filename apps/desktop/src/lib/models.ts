export type ModelState =
  "available" | "downloading" | "paused" | "installed" | "failed";

export type ModelItem = {
  packageId: string;
  profileId: string | null;
  displayName: string;
  summary: string;
  languages: string[];
  licenseId: string;
  totalBytes: number;
  deviceKind: string;
  minRamMb: number | null;
  minVramMb: number | null;
  recommended: boolean;
  ownerMeasured: boolean;
  state: ModelState;
  isActive: boolean;
  bytesDownloaded: number;
  healthState: string | null;
  inCatalog: boolean;
  /// Package id that already holds these exact weights, if any.
  duplicateOf: string | null;
};

export type ModelsView = {
  items: ModelItem[];
  activeProfileId: string | null;
  catalogError: string | null;
  busyPackageId: string | null;
};

export type ModelProgress = {
  packageId: string;
  bytesDownloaded: number;
  totalBytes: number;
};

const LANGUAGE_NAMES: Record<string, string> = {
  // Все whisper-модели каталога многоязычные; перечислять 99 кодов бессмысленно.
  multi: "Многоязычная",
  ru: "Русский",
  en: "English",
};

export function languageLabel(code: string): string {
  return LANGUAGE_NAMES[code] ?? code.toUpperCase();
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "—";
  const megabytes = bytes / (1024 * 1024);
  if (megabytes >= 1024) {
    return `${(megabytes / 1024).toFixed(1)} ГБ`;
  }
  return `${Math.round(megabytes)} МБ`;
}

export function deviceLabel(deviceKind: string): string {
  if (deviceKind === "vulkan") return "GPU (Vulkan)";
  if (deviceKind === "directml") return "GPU (DirectML)";
  if (deviceKind === "cpu") return "CPU";
  return deviceKind.length > 0 ? deviceKind : "—";
}

export function requirementLabel(item: ModelItem): string {
  const parts = [deviceLabel(item.deviceKind)];
  if (item.minRamMb)
    parts.push(`ОЗУ от ${formatBytes(item.minRamMb * 1048576)}`);
  if (item.minVramMb) {
    parts.push(`видеопамять от ${formatBytes(item.minVramMb * 1048576)}`);
  }
  return parts.join(" · ");
}

export function stateLabel(item: ModelItem): string {
  switch (item.state) {
    case "installed":
      return item.isActive ? "Активная модель" : "Установлена";
    case "downloading":
      return "Загружается";
    case "paused":
      return "Загрузка приостановлена";
    case "failed":
      return "Установка не удалась";
    default:
      return "Доступна для загрузки";
  }
}

/// Only a model this project actually ran may carry a quality claim. For the rest we say plainly
/// that we have no numbers rather than inventing a rating.
export function measurementLabel(item: ModelItem): string {
  return item.ownerMeasured ? "Измерена нами" : "Наших измерений нет";
}

/// Фильтра по языку нет: в каталоге одни многоязычные модели, и выбор языка ничего не отсекал.
/// Он вернётся вместе с первой моделью на ограниченный набор языков.
export function filterModels(items: ModelItem[], query: string): ModelItem[] {
  const needle = query.trim().toLocaleLowerCase("ru");
  if (needle.length === 0) return [...items];
  return items.filter((item) =>
    item.displayName.toLocaleLowerCase("ru").includes(needle),
  );
}

/// Installed first, the active one at the very top: the model in use is the thing a user looks
/// for, and a fresh install should not push it below a list of downloads.
export function sortModels(items: ModelItem[]): ModelItem[] {
  const rank = (item: ModelItem): number => {
    if (item.isActive) return 0;
    if (item.state === "installed") return 1;
    if (item.state === "downloading" || item.state === "paused") return 2;
    if (item.recommended) return 3;
    return 4;
  };
  return [...items].sort(
    (left, right) =>
      rank(left) - rank(right) ||
      left.displayName.localeCompare(right.displayName, "ru"),
  );
}

export function downloadedBytes(
  item: ModelItem,
  progress: ModelProgress | undefined,
): number {
  if (progress && progress.packageId === item.packageId) {
    return progress.bytesDownloaded;
  }
  return item.bytesDownloaded;
}

export function progressPercent(
  item: ModelItem,
  progress: ModelProgress | undefined,
): number {
  if (item.totalBytes <= 0) return 0;
  const done = downloadedBytes(item, progress);
  return Math.min(100, Math.max(0, Math.round((done / item.totalBytes) * 100)));
}

/// The import folder is the only absolute path from outside the managed root that the app
/// accepts, so it is checked here before it ever reaches the installer. The installer verifies
/// the same signature and checksums it would for a download; this only catches typos early.
export function validateImportDirectory(value: string): string | null {
  if (!value) return "Укажите папку, где лежат файлы модели.";
  if (value.length > 1024 || value.includes("\0"))
    return "Путь слишком длинный или содержит недопустимый символ.";
  if (!/^[a-zA-Z]:\\/.test(value) && !/^\\\\[^\\]+\\[^\\]+\\/.test(value))
    return "Нужен полный Windows-путь, например D:\\models\\whisper-base.";
  return null;
}

export function canActivate(item: ModelItem): boolean {
  return (
    item.state === "installed" &&
    !item.isActive &&
    item.profileId !== null &&
    item.healthState === "healthy"
  );
}

/// Removal frees bytes, so it is offered only where bytes exist: an installed package, or an
/// interrupted download whose partial file is still on disk. The active model is never removable
/// - that would leave dictation with no runtime.
export function canRemove(item: ModelItem): boolean {
  if (item.isActive || item.state === "downloading") return false;
  return (
    item.state === "installed" ||
    item.state === "paused" ||
    item.state === "failed"
  );
}

/// Downloading is pointless when the very same weights already sit on disk under another
/// package: it would spend hundreds of megabytes to duplicate them.
export function canDownload(item: ModelItem): boolean {
  if (!item.inCatalog || item.duplicateOf) return false;
  return item.state === "available" || item.state === "failed";
}

export function hasInstalledModel(items: ModelItem[]): boolean {
  return items.some((item) => item.state === "installed");
}

/// What a screen with nothing on it should say. A fresh install must read as "start here", never
/// as a broken window.
export function emptyStateMessage(view: ModelsView): string | null {
  if (hasInstalledModel(view.items)) return null;
  if (view.items.length > 0) {
    return "Ни одна модель ещё не установлена. Выберите модель из списка и нажмите «Скачать» — без неё распознавание не заработает.";
  }
  return "Ни одна модель ещё не установлена, а каталог сейчас недоступен — скачать новую пока неоткуда.";
}
