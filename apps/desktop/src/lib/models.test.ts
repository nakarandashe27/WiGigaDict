import { describe, expect, it } from "vitest";
import {
  canActivate,
  canDownload,
  canRemove,
  emptyStateMessage,
  filterModels,
  formatBytes,
  languageLabel,
  measurementLabel,
  progressPercent,
  requirementLabel,
  sortModels,
  stateLabel,
  validateImportDirectory,
  type ModelItem,
  type ModelsView,
} from "./models";

function model(overrides: Partial<ModelItem> = {}): ModelItem {
  return {
    packageId: "whisper-small-cpu",
    profileId: "whisper-small-cpu",
    displayName: "Whisper small",
    summary: "",
    languages: ["ru", "en"],
    licenseId: "MIT",
    totalBytes: 487601967,
    deviceKind: "cpu",
    minRamMb: 4096,
    minVramMb: null,
    recommended: false,
    ownerMeasured: false,
    state: "available",
    isActive: false,
    bytesDownloaded: 0,
    healthState: null,
    inCatalog: true,
    duplicateOf: null,
    ...overrides,
  };
}

function view(overrides: Partial<ModelsView> = {}): ModelsView {
  return {
    items: [],
    activeProfileId: null,
    catalogError: null,
    busyPackageId: null,
    ...overrides,
  };
}

describe("model catalog presentation", () => {
  it("reports sizes the way a download dialog would", () => {
    expect(formatBytes(147951465)).toBe("141 МБ");
    expect(formatBytes(574041195)).toBe("547 МБ");
    expect(formatBytes(2 * 1024 ** 3)).toBe("2.0 ГБ");
    expect(formatBytes(0)).toBe("—");
  });

  it("spells out hardware requirements including video memory", () => {
    expect(requirementLabel(model())).toBe("CPU · ОЗУ от 4.0 ГБ");
    expect(
      requirementLabel(
        model({ deviceKind: "vulkan", minRamMb: 2048, minVramMb: 1024 }),
      ),
    ).toBe("GPU (Vulkan) · ОЗУ от 2.0 ГБ · видеопамять от 1.0 ГБ");
  });

  it("names a multilingual model instead of listing a couple of codes", () => {
    expect(languageLabel("multi")).toBe("Многоязычная");
    expect(languageLabel("ru")).toBe("Русский");
    expect(languageLabel("de")).toBe("DE");
  });

  it("never implies a quality claim for a model we did not run", () => {
    expect(measurementLabel(model())).toBe("Наших измерений нет");
    expect(measurementLabel(model({ ownerMeasured: true }))).toBe(
      "Измерена нами",
    );
  });

  it("separates an interrupted download from a failure", () => {
    expect(stateLabel(model({ state: "paused" }))).toBe(
      "Загрузка приостановлена",
    );
    expect(stateLabel(model({ state: "failed" }))).toBe("Установка не удалась");
    expect(stateLabel(model({ state: "installed", isActive: true }))).toBe(
      "Активная модель",
    );
  });
});

describe("model list filtering", () => {
  const items = [
    model({ packageId: "a", displayName: "Whisper base", languages: ["ru"] }),
    model({ packageId: "b", displayName: "Vosk small", languages: ["ru"] }),
    model({ packageId: "c", displayName: "Parakeet EN", languages: ["en"] }),
  ];

  it("matches names case-insensitively and keeps the whole list when empty", () => {
    expect(filterModels(items, "whisper").map((i) => i.packageId)).toEqual([
      "a",
    ]);
    expect(filterModels(items, "WHISPER")).toHaveLength(1);
    expect(filterModels(items, "  ")).toHaveLength(3);
    expect(filterModels(items, "kaldi")).toHaveLength(0);
  });

  it("puts the model in use first and downloads above the rest", () => {
    const sorted = sortModels([
      model({ packageId: "available", displayName: "Zeta" }),
      model({ packageId: "downloading", state: "downloading" }),
      model({ packageId: "active", state: "installed", isActive: true }),
      model({ packageId: "installed", state: "installed" }),
    ]);
    expect(sorted.map((item) => item.packageId)).toEqual([
      "active",
      "installed",
      "downloading",
      "available",
    ]);
  });
});

describe("model actions", () => {
  it("only activates an installed healthy profile that is not already active", () => {
    expect(
      canActivate(model({ state: "installed", healthState: "healthy" })),
    ).toBe(true);
    expect(
      canActivate(
        model({ state: "installed", healthState: "healthy", isActive: true }),
      ),
    ).toBe(false);
    expect(
      canActivate(model({ state: "installed", healthState: "unhealthy" })),
    ).toBe(false);
    expect(canActivate(model({ state: "available" }))).toBe(false);
  });

  it("never offers to remove the active model or a running download", () => {
    expect(canRemove(model({ state: "installed", isActive: true }))).toBe(
      false,
    );
    expect(canRemove(model({ state: "downloading" }))).toBe(false);
    expect(canRemove(model({ state: "installed" }))).toBe(true);
  });

  it("does not offer a download of weights that are already on disk", () => {
    expect(canDownload(model({ state: "available" }))).toBe(true);
    expect(
      canDownload(model({ state: "available", duplicateOf: "other-package" })),
    ).toBe(false);
    // A package the catalog does not list has nothing to download from.
    expect(canDownload(model({ state: "available", inCatalog: false }))).toBe(
      false,
    );
  });

  it("offers removal only where bytes actually exist on disk", () => {
    // A model that was never downloaded has nothing to delete.
    expect(canRemove(model({ state: "available" }))).toBe(false);
    // An interrupted download left a partial file, so freeing it is meaningful.
    expect(canRemove(model({ state: "paused" }))).toBe(true);
    expect(canRemove(model({ state: "failed" }))).toBe(true);
  });

  it("tracks progress from the live event and falls back to stored bytes", () => {
    const item = model({ state: "downloading", bytesDownloaded: 100000000 });
    expect(progressPercent(item, undefined)).toBe(21);
    expect(
      progressPercent(item, {
        packageId: item.packageId,
        bytesDownloaded: 243800983,
        totalBytes: item.totalBytes,
      }),
    ).toBe(50);
    // An event about another package must not move this card's bar.
    expect(
      progressPercent(item, {
        packageId: "other",
        bytesDownloaded: 400000000,
        totalBytes: item.totalBytes,
      }),
    ).toBe(21);
  });
});

describe("empty state", () => {
  it("points a fresh install at the catalog instead of looking broken", () => {
    const message = emptyStateMessage(view({ items: [model()] }));
    expect(message).toContain("Скачать");
  });

  it("says plainly that nothing can be installed when the catalog is gone", () => {
    const message = emptyStateMessage(view({ catalogError: "нет ключа" }));
    expect(message).toContain("неоткуда");
  });

  it("says nothing once a model is installed", () => {
    expect(
      emptyStateMessage(view({ items: [model({ state: "installed" })] })),
    ).toBeNull();
  });
});

describe("local import path", () => {
  it("accepts a full drive path and a UNC share", () => {
    expect(validateImportDirectory("D:\\models\\whisper-base")).toBeNull();
    expect(validateImportDirectory("\\\\nas\\models\\whisper")).toBeNull();
  });

  it("refuses anything that is not a full Windows path", () => {
    expect(validateImportDirectory("")).toContain("папку");
    expect(validateImportDirectory("models\\whisper")).toContain("полный");
    expect(validateImportDirectory("/home/user/models")).toContain("полный");
  });

  it("refuses an oversized path or an embedded null", () => {
    expect(validateImportDirectory("D:\\" + "a".repeat(1200))).toContain(
      "слишком длинный",
    );
    expect(validateImportDirectory("D:\\models\\a\0b")).toContain(
      "недопустимый",
    );
  });
});
