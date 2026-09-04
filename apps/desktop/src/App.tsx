import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowsClockwise,
  ArrowCounterClockwise,
  CheckCircle,
  ClockCounterClockwise,
  Copy,
  Cpu,
  Cube,
  GearSix,
  FolderOpen,
  MagnifyingGlass,
  Microphone,
  PushPin,
  PushPinSlash,
  SlidersHorizontal,
  Trash,
  WarningCircle,
  Waveform,
} from "@phosphor-icons/react";

import {
  audioGapWarning,
  canCancelCapture,
  captureLabel,
  initialCaptureStatus,
  type CaptureStatus,
} from "./lib/capture-status";
import {
  canDelete,
  canRetry,
  recoveryLabel,
  type RecoveryEntry,
} from "./lib/recovery";
import {
  configurationUpdate,
  runtimeLabel,
  validateConfiguration,
  type ConfigurationUpdate,
  type SettingsView,
} from "./lib/settings";
import {
  DIAGNOSTIC_EXPORT_CONFIRMATION,
  formatDiagnosticBytes,
  validateDiagnosticDestination,
  type DiagnosticBundlePreview,
  type DiagnosticExportReceipt,
  type DiagnosticView,
} from "./lib/diagnostics";
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
  type ModelProgress,
  type ModelsView,
} from "./lib/models";
import { nextDialogControl, type DialogControl } from "./lib/keyboard";
import type { OverlayStatus } from "./lib/overlay-status";
import { APP_VERSION } from "./lib/version";

type Section = "dictation" | "history" | "models" | "settings";

type RuntimeStatus = {
  state: "ready" | "processing" | "unavailable";
  protocol: string;
  sidecar: string;
  detail: string;
};

type Confirmation = {
  kind: "retry" | "delete";
  entry: RecoveryEntry;
};

type Notice = {
  tone: "info" | "success" | "warning";
  message: string;
};

const initialRuntimeStatus: RuntimeStatus = {
  state: "unavailable",
  protocol: "—",
  sidecar: "—",
  detail: "Native shell is not ready",
};

const sections: Array<{ id: Section; label: string; icon: ReactNode }> = [
  {
    id: "dictation",
    label: "Диктовка",
    icon: <Microphone aria-hidden="true" size={21} weight="regular" />,
  },
  {
    id: "history",
    label: "История",
    icon: (
      <ClockCounterClockwise aria-hidden="true" size={21} weight="regular" />
    ),
  },
  {
    id: "models",
    label: "Модели",
    icon: <Cpu aria-hidden="true" size={21} weight="regular" />,
  },
  {
    id: "settings",
    label: "Настройки",
    icon: <SlidersHorizontal aria-hidden="true" size={21} weight="regular" />,
  },
];

function App() {
  const [activeSection, setActiveSection] = useState<Section>("dictation");
  const [runtime, setRuntime] = useState<RuntimeStatus>(initialRuntimeStatus);
  const [capture, setCapture] = useState<CaptureStatus>(initialCaptureStatus);
  const [overlay, setOverlay] = useState<OverlayStatus | null>(null);
  const [history, setHistory] = useState<RecoveryEntry[]>([]);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [models, setModels] = useState<ModelsView | null>(null);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [modelProgress, setModelProgress] = useState<ModelProgress | undefined>(
    undefined,
  );
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [busySession, setBusySession] = useState<string | null>(null);
  const [savingSettings, setSavingSettings] = useState(false);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);

  useEffect(() => {
    let disposed = false;
    void Promise.allSettled([
      invoke<RuntimeStatus>("runtime_status"),
      invoke<CaptureStatus>("capture_status"),
      invoke<RecoveryEntry[]>("recovery_list"),
      invoke<SettingsView>("settings_get"),
      invoke<ModelsView>("models_list"),
    ]).then(
      ([
        runtimeResult,
        captureResult,
        historyResult,
        settingsResult,
        modelsResult,
      ]) => {
        if (disposed) return;
        if (runtimeResult.status === "fulfilled")
          setRuntime(runtimeResult.value);
        if (captureResult.status === "fulfilled")
          setCapture(captureResult.value);
        if (historyResult.status === "fulfilled")
          setHistory(historyResult.value);
        else setHistoryError(String(historyResult.reason));
        if (settingsResult.status === "fulfilled")
          setSettings(settingsResult.value);
        else setSettingsError(String(settingsResult.reason));
        if (modelsResult.status === "fulfilled") setModels(modelsResult.value);
        else setModelsError(String(modelsResult.reason));
      },
    );
    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;

    let disposed = false;
    let stopListening: Array<() => void> = [];
    void Promise.all([
      listen<CaptureStatus>("capture-status", (event) => {
        if (!disposed) setCapture(event.payload);
      }),
      listen<OverlayStatus>("overlay-status", (event) => {
        if (!disposed) setOverlay(event.payload);
      }),
      listen<ModelProgress>("models-progress", (event) => {
        if (!disposed) setModelProgress(event.payload);
      }),
      listen<null>("models-changed", () => {
        if (!disposed) {
          setModelProgress(undefined);
          void refreshModels();
        }
      }),
      listen<string>("models-error", (event) => {
        if (!disposed) setModelsError(event.payload);
      }),
      listen<string>("shell-navigate", (event) => {
        if (
          !disposed &&
          (event.payload === "dictation" ||
            event.payload === "history" ||
            event.payload === "models" ||
            event.payload === "settings")
        ) {
          setActiveSection(event.payload);
        }
      }),
    ]).then((unlisteners) => {
      if (disposed) unlisteners.forEach((unlisten) => unlisten());
      else stopListening = unlisteners;
    });
    return () => {
      disposed = true;
      stopListening.forEach((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (!canCancelCapture(capture.phase)) return;
    const cancelOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.repeat) {
        event.preventDefault();
        void invoke("cancel_capture");
      }
    };
    window.addEventListener("keydown", cancelOnEscape);
    return () => window.removeEventListener("keydown", cancelOnEscape);
  }, [capture.phase]);

  async function refreshHistory() {
    setHistory(await invoke<RecoveryEntry[]>("recovery_list"));
  }

  async function refreshModels() {
    try {
      setModels(await invoke<ModelsView>("models_list"));
    } catch (error: unknown) {
      setModelsError(String(error));
    }
  }

  async function modelAction(operation: () => Promise<unknown>) {
    setModelsError(null);
    try {
      await operation();
      await refreshModels();
    } catch (error: unknown) {
      setModelsError(String(error));
    }
  }

  async function refreshSettings() {
    setSettingsError(null);
    try {
      setSettings(await invoke<SettingsView>("settings_get"));
    } catch (error: unknown) {
      setSettingsError(String(error));
    }
  }

  async function perform(
    entry: RecoveryEntry,
    operation: (actionId: string) => Promise<unknown>,
    successMessage: string,
  ) {
    setBusySession(entry.sessionId);
    setHistoryError(null);
    setNotice(null);
    try {
      await operation(crypto.randomUUID());
      await refreshHistory();
      setNotice({ tone: "success", message: successMessage });
    } catch (error: unknown) {
      setHistoryError(String(error));
      await refreshHistory().catch(() => undefined);
    } finally {
      setBusySession(null);
    }
  }

  function copy(entry: RecoveryEntry) {
    if (!entry.selected) return;
    void perform(
      entry,
      async (actionId) => {
        await navigator.clipboard.writeText(entry.selected?.content ?? "");
        return invoke("recovery_record_copy", {
          sessionId: entry.sessionId,
          expectedStateVersion: entry.stateVersion,
          actionId,
        });
      },
      "Текст скопирован.",
    );
  }

  function resolve(entry: RecoveryEntry) {
    void perform(
      entry,
      (actionId) =>
        invoke("recovery_resolve", {
          sessionId: entry.sessionId,
          expectedStateVersion: entry.stateVersion,
          actionId,
        }),
      "Результат отмечен как решённый.",
    );
  }

  function setPinned(entry: RecoveryEntry) {
    void perform(
      entry,
      (actionId) =>
        invoke("recovery_set_pinned", {
          sessionId: entry.sessionId,
          expectedStateVersion: entry.stateVersion,
          actionId,
          pinned: !entry.pinned,
        }),
      entry.pinned ? "Результат откреплён." : "Результат закреплён.",
    );
  }

  function confirmAction() {
    if (!confirmation) return;
    const { entry, kind } = confirmation;
    setConfirmation(null);
    if (kind === "retry") {
      void perform(
        entry,
        (actionId) =>
          invoke("recovery_retry", {
            sessionId: entry.sessionId,
            expectedStateVersion: entry.stateVersion,
            actionId,
          }),
        "Повторная вставка завершена. Проверьте активное поле.",
      );
      return;
    }
    void perform(
      entry,
      (actionId) =>
        invoke("recovery_delete", {
          sessionId: entry.sessionId,
          expectedStateVersion: entry.stateVersion,
          actionId,
        }),
      "Локальный результат удалён.",
    );
  }

  async function saveSettings(update: ConfigurationUpdate) {
    setSavingSettings(true);
    setSettingsError(null);
    setNotice(null);
    try {
      setSettings(await invoke<SettingsView>("settings_update", { update }));
      setNotice({ tone: "success", message: "Настройки сохранены." });
    } catch (error: unknown) {
      setSettingsError(String(error));
      await refreshSettings();
    } finally {
      setSavingSettings(false);
    }
  }

  return (
    <main className="app-shell">
      <Navigation active={activeSection} onNavigate={setActiveSection} />
      <section className="workspace">
        <header className="topbar">
          <div>
            <h1>
              {sections.find((section) => section.id === activeSection)?.label}
            </h1>
          </div>
          <LiveBadge capture={capture} overlay={overlay} />
        </header>

        {notice ? (
          <div className={"notice notice-" + notice.tone} role="status">
            {notice.message}
          </div>
        ) : null}

        {activeSection === "dictation" ? (
          <DictationSection
            capture={capture}
            runtime={runtime}
            settings={settings}
            onOpenSettings={() => setActiveSection("settings")}
            onOpenHistory={() => setActiveSection("history")}
          />
        ) : null}

        {activeSection === "history" ? (
          <HistorySection
            entries={history}
            busySession={busySession}
            error={historyError}
            onRefresh={() => void refreshHistory()}
            onRetry={(entry) => setConfirmation({ kind: "retry", entry })}
            onCopy={copy}
            onResolve={resolve}
            onSetPinned={setPinned}
            onDelete={(entry) => setConfirmation({ kind: "delete", entry })}
          />
        ) : null}

        {activeSection === "models" ? (
          <ModelsSection
            value={models}
            error={modelsError}
            progress={modelProgress}
            onRefresh={() => void refreshModels()}
            onInstall={(item) =>
              void modelAction(() =>
                invoke("models_install_start", { packageId: item.packageId }),
              )
            }
            onPause={() =>
              void modelAction(() => invoke("models_install_pause"))
            }
            onCancel={() =>
              void modelAction(() => invoke("models_install_cancel"))
            }
            onActivate={(item) =>
              void modelAction(() =>
                invoke("models_activate", { profileId: item.profileId }),
              )
            }
            onRemove={(item) =>
              void modelAction(() =>
                invoke("models_remove", { packageId: item.packageId }),
              )
            }
            onImport={(item, directory) =>
              void modelAction(() =>
                invoke("models_import_local", {
                  packageId: item.packageId,
                  sourceDirectory: directory,
                }),
              )
            }
          />
        ) : null}

        {activeSection === "settings" ? (
          <SettingsSection
            value={settings}
            error={settingsError}
            saving={savingSettings}
            onReload={() => void refreshSettings()}
            onSave={(update) => void saveSettings(update)}
          />
        ) : null}
      </section>

      {confirmation ? (
        <ConfirmDialog
          confirmation={confirmation}
          onCancel={() => setConfirmation(null)}
          onConfirm={confirmAction}
        />
      ) : null}
    </main>
  );
}

function ModelsSection({
  value,
  error,
  progress,
  onRefresh,
  onInstall,
  onPause,
  onCancel,
  onActivate,
  onRemove,
  onImport,
}: {
  value: ModelsView | null;
  error: string | null;
  progress: ModelProgress | undefined;
  onRefresh: () => void;
  onInstall: (item: ModelItem) => void;
  onPause: () => void;
  onCancel: () => void;
  onActivate: (item: ModelItem) => void;
  onRemove: (item: ModelItem) => void;
  onImport: (item: ModelItem, directory: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [pendingRemoval, setPendingRemoval] = useState<string | null>(null);
  const [importPaths, setImportPaths] = useState<Record<string, string>>({});
  const [importError, setImportError] = useState<string | null>(null);

  if (!value) {
    return (
      <section className="section-card">
        <p>Загружаем список моделей…</p>
        {error ? (
          <p className="error-notice" role="alert">
            {error}
          </p>
        ) : null}
      </section>
    );
  }

  const visible = sortModels(filterModels(value.items, query));
  const empty = emptyStateMessage(value);
  const busy = value.busyPackageId !== null;

  return (
    <section className="section-card">
      <div className="section-heading">
        <div>
          <p className="section-label">Локальные модели</p>
          <h2>Модели распознавания</h2>
        </div>
        <button
          type="button"
          className="button button-ghost"
          onClick={onRefresh}
        >
          Обновить
        </button>
      </div>

      <p className="models-network-note">
        Скачивание моделей — единственное действие WiGigaDict, которому нужен
        интернет. Распознавание, вставка и история всегда остаются локальными.
      </p>

      {error ? (
        <p className="error-notice" role="alert">
          {error}
        </p>
      ) : null}

      {value.catalogError ? (
        <p className="error-notice" role="status">
          Каталог моделей недоступен: {value.catalogError}. Уже установленные
          модели продолжают работать.
        </p>
      ) : null}

      {empty ? <p className="empty-state">{empty}</p> : null}

      <div className="models-filters">
        <label className="field models-search-field">
          <span>Поиск по имени</span>
          <span className="search-input">
            <MagnifyingGlass aria-hidden="true" size={18} weight="regular" />
            <input
              type="search"
              value={query}
              placeholder="Например, Whisper"
              onChange={(event) => setQuery(event.target.value)}
            />
          </span>
        </label>
      </div>

      {visible.length === 0 && value.items.length > 0 ? (
        <p>По этому запросу ничего не найдено.</p>
      ) : null}

      <ul className="model-list">
        {visible.map((item) => (
          <li
            className={
              "model-card" + (item.isActive ? " model-card-active" : "")
            }
            key={item.packageId}
          >
            <header className="model-card-heading">
              <div className="model-title">
                <span className="model-icon" aria-hidden="true">
                  <Cube size={22} weight="regular" />
                </span>
                <h3>{item.displayName}</h3>
              </div>
              <div className="model-badges">
                {item.isActive ? (
                  <span className="badge badge-active">Активная</span>
                ) : null}
                {item.recommended && !item.isActive ? (
                  <span className="badge">Рекомендуется</span>
                ) : null}
                <span
                  className={
                    "badge " +
                    (item.ownerMeasured ? "badge-measured" : "badge-unmeasured")
                  }
                >
                  {measurementLabel(item)}
                </span>
              </div>
            </header>

            {item.summary ? (
              <p className="model-summary">{item.summary}</p>
            ) : null}

            <dl className="model-meta">
              <div>
                <dt>Размер</dt>
                <dd>{formatBytes(item.totalBytes)}</dd>
              </div>
              <div>
                <dt>Языки</dt>
                <dd>
                  {item.languages.length > 0
                    ? item.languages.map(languageLabel).join(", ")
                    : "—"}
                </dd>
              </div>
              <div>
                <dt>Лицензия</dt>
                <dd>{item.licenseId}</dd>
              </div>
              <div>
                <dt>Требования</dt>
                <dd>{requirementLabel(item)}</dd>
              </div>
            </dl>

            <p className="model-state">
              {stateLabel(item)}
              {item.inCatalog ? "" : " · установлена вне каталога"}
            </p>

            {item.duplicateOf ? (
              <p className="model-hint">
                Эти веса уже установлены в пакете «{item.duplicateOf}» —
                скачивать их повторно не нужно.
              </p>
            ) : null}

            {item.state === "downloading" || item.state === "paused" ? (
              <div className="model-progress">
                <progress max={100} value={progressPercent(item, progress)} />
                <span>{progressPercent(item, progress)}%</span>
              </div>
            ) : null}

            <div className="model-actions">
              {canDownload(item) ? (
                <button
                  type="button"
                  className="button button-primary"
                  disabled={busy}
                  onClick={() => onInstall(item)}
                >
                  Скачать
                </button>
              ) : null}

              {item.state === "paused" ? (
                <button
                  type="button"
                  className="button button-primary"
                  disabled={busy}
                  onClick={() => onInstall(item)}
                >
                  Продолжить
                </button>
              ) : null}

              {item.state === "downloading" ? (
                <>
                  <button
                    type="button"
                    className="button"
                    onClick={() => onPause()}
                  >
                    Пауза
                  </button>
                  <button
                    type="button"
                    className="button button-ghost"
                    onClick={() => onCancel()}
                  >
                    Отменить загрузку
                  </button>
                </>
              ) : null}

              {canActivate(item) ? (
                <button
                  type="button"
                  className="button"
                  onClick={() => onActivate(item)}
                >
                  Сделать активной
                </button>
              ) : null}

              {canRemove(item) ? (
                pendingRemoval === item.packageId ? (
                  <span className="model-confirm">
                    Удалить файлы модели с диска?
                    <button
                      type="button"
                      className="button button-danger"
                      onClick={() => {
                        setPendingRemoval(null);
                        onRemove(item);
                      }}
                    >
                      Удалить
                    </button>
                    <button
                      type="button"
                      className="button button-ghost"
                      onClick={() => setPendingRemoval(null)}
                    >
                      Отмена
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    className="button button-ghost"
                    onClick={() => setPendingRemoval(item.packageId)}
                  >
                    Удалить
                  </button>
                )
              ) : null}

              {item.isActive ? (
                <span className="model-hint">
                  Активную модель удалить нельзя — сначала выберите другую.
                </span>
              ) : null}
            </div>

            {item.inCatalog &&
            !item.duplicateOf &&
            item.state !== "installed" &&
            item.state !== "downloading" ? (
              <details className="model-import">
                <summary>Установить из папки</summary>
                <label className="field">
                  <span>Папка с уже скачанными файлами модели</span>
                  <input
                    autoComplete="off"
                    onChange={(event) => {
                      setImportError(null);
                      setImportPaths((current) => ({
                        ...current,
                        [item.packageId]: event.target.value,
                      }));
                    }}
                    placeholder="D:\models\whisper-base"
                    spellCheck={false}
                    value={importPaths[item.packageId] ?? ""}
                  />
                  <small>
                    Файлы проверяются той же подписью и контрольными суммами,
                    что и загрузка.
                  </small>
                </label>
                <button
                  className="button"
                  disabled={busy}
                  onClick={() => {
                    const directory = (
                      importPaths[item.packageId] ?? ""
                    ).trim();
                    const problem = validateImportDirectory(directory);
                    setImportError(problem);
                    if (!problem) onImport(item, directory);
                  }}
                  type="button"
                >
                  Установить
                </button>
                {importError ? (
                  <p className="form-error">{importError}</p>
                ) : null}
              </details>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}

function Navigation({
  active,
  onNavigate,
}: {
  active: Section;
  onNavigate: (section: Section) => void;
}) {
  return (
    <nav className="navigation" aria-label="Основные разделы">
      <div className="brand-lockup" aria-label="WiGigaDict">
        <span className="brand-mark" aria-hidden="true">
          <Waveform size={19} weight="bold" />
        </span>
        <span className="brand-name">WiGigaDict</span>
      </div>
      <div className="nav-items">
        {sections.map((section) => (
          <button
            aria-current={active === section.id ? "page" : undefined}
            className="nav-item"
            key={section.id}
            onClick={() => onNavigate(section.id)}
            type="button"
          >
            <span className="nav-icon" aria-hidden="true">
              {section.icon}
            </span>
            {section.label}
          </button>
        ))}
      </div>
      <div className="version">v{APP_VERSION}</div>
    </nav>
  );
}

function LiveBadge({
  capture,
  overlay,
}: {
  capture: CaptureStatus;
  overlay: OverlayStatus | null;
}) {
  const phase =
    capture.phase === "recording" || capture.phase === "preparing"
      ? capture.phase
      : (overlay?.phase ?? capture.phase);
  const label =
    phase === "processing"
      ? "Обработка"
      : phase === "delivered"
        ? "Доставлено"
        : phase === "uncertain"
          ? "Recovery"
          : captureLabel(capture.phase);
  return (
    <div className={"live-badge phase-" + phase} role="status">
      <span className="live-badge-dot" aria-hidden="true" />
      <strong>{label}</strong>
    </div>
  );
}

function DictationSection({
  capture,
  runtime,
  settings,
  onOpenSettings,
  onOpenHistory,
}: {
  capture: CaptureStatus;
  runtime: RuntimeStatus;
  settings: SettingsView | null;
  onOpenSettings: () => void;
  onOpenHistory: () => void;
}) {
  const configuration = settings?.configuration;
  const runtimeProfile = settings?.runtimeProfiles.find(
    (profile) => profile.id === configuration?.activeRuntimeProfileId,
  );
  const microphone = settings?.inputDevices.find(
    (device) => device.id === configuration?.microphoneDeviceId,
  );
  const gapWarning = audioGapWarning(capture);

  return (
    <div className="content-stack">
      {gapWarning ? (
        <p className="error-notice" role="status">
          {gapWarning}
        </p>
      ) : null}
      <section className="hero-card" aria-labelledby="dictation-title">
        <div className="hero-copy">
          <div className="hero-icon" aria-hidden="true">
            <Microphone size={36} weight="light" />
          </div>
          <h2 id="dictation-title">Нажмите для начала диктовки</h2>
          <p>
            Используйте <kbd>{configuration?.hotkeyBinding ?? "F8"}</kbd>,
            говорите естественно и нажмите сочетание ещё раз. Запись и
            распознавание остаются на этом компьютере.
          </p>
          <div className="hero-actions">
            <button
              className="button button-primary"
              onClick={onOpenSettings}
              type="button"
            >
              Настроить диктовку
            </button>
            <button
              className="button button-ghost"
              onClick={onOpenHistory}
              type="button"
            >
              Открыть recovery
            </button>
          </div>
        </div>
        <div className={"capture-orbit phase-" + capture.phase}>
          <span className="orbit-core" aria-hidden="true">
            <Waveform size={22} weight="regular" />
          </span>
          <strong>{captureLabel(capture.phase)}</strong>
          <small>
            {capture.reason
              ? friendlyReason(capture.reason)
              : "Результат остаётся на этом компьютере"}
          </small>
        </div>
      </section>

      <section className="status-grid" aria-label="Готовность диктовки">
        <StatusCard
          label="Микрофон"
          icon={<Microphone aria-hidden="true" size={20} weight="regular" />}
          state={
            microphone?.name ??
            (configuration?.microphoneDeviceId
              ? "Выбранное устройство недоступно"
              : "Системный по умолчанию")
          }
          tone={capture.deviceHealthy ? "ready" : "neutral"}
        />
        <StatusCard
          label="Локальная модель"
          icon={<Cpu aria-hidden="true" size={20} weight="regular" />}
          state={
            runtimeProfile
              ? runtimeLabel(runtimeProfile)
              : "Установленная модель не выбрана"
          }
          tone={runtimeProfile?.available ? "ready" : "warning"}
        />
        <StatusCard
          label="Обработчик"
          icon={<GearSix aria-hidden="true" size={20} weight="regular" />}
          state={
            runtime.state === "ready"
              ? "Готов"
              : runtime.state === "processing"
                ? "Обрабатывает запись"
                : "Недоступен"
          }
          tone={runtime.state === "ready" ? "ready" : "warning"}
        />
      </section>

      <details className="technical-details">
        <summary>Техническое состояние</summary>
        <dl>
          <div>
            <dt>Протокол</dt>
            <dd>{runtime.protocol}</dd>
          </div>
          <div>
            <dt>Sidecar</dt>
            <dd>{runtime.sidecar}</dd>
          </div>
          <div>
            <dt>Состояние</dt>
            <dd>{runtime.detail}</dd>
          </div>
        </dl>
      </details>
    </div>
  );
}

function StatusCard({
  label,
  state,
  tone,
  icon,
}: {
  label: string;
  state: string;
  tone: "ready" | "warning" | "neutral";
  icon: ReactNode;
}) {
  return (
    <article className={"status-card status-card-" + tone}>
      <span className="status-icon">{icon}</span>
      <p>{label}</p>
      <strong>{state}</strong>
    </article>
  );
}

function HistorySection({
  entries,
  busySession,
  error,
  onRefresh,
  onRetry,
  onCopy,
  onResolve,
  onSetPinned,
  onDelete,
}: {
  entries: RecoveryEntry[];
  busySession: string | null;
  error: string | null;
  onRefresh: () => void;
  onRetry: (entry: RecoveryEntry) => void;
  onCopy: (entry: RecoveryEntry) => void;
  onResolve: (entry: RecoveryEntry) => void;
  onSetPinned: (entry: RecoveryEntry) => void;
  onDelete: (entry: RecoveryEntry) => void;
}) {
  return (
    <div className="content-stack">
      <section className="section-card">
        <div className="section-heading">
          <div>
            <p className="section-label">Локальные результаты</p>
            <h2>История и recovery</h2>
            <p>
              Неопределённая вставка остаётся здесь, пока вы не решите, что
              делать с результатом.
            </p>
          </div>
          <button
            className="button button-ghost"
            onClick={onRefresh}
            type="button"
          >
            <ArrowsClockwise
              className="button-leading-icon"
              aria-hidden="true"
              size={18}
              weight="bold"
            />
            Обновить
          </button>
        </div>

        {error ? <ErrorNotice detail={error} /> : null}

        {entries.length === 0 ? (
          <div className="empty-state">
            <span className="empty-signal" aria-hidden="true" />
            <strong>Локальных результатов пока нет</strong>
            <p>После первой диктовки здесь появится подтверждённая история.</p>
          </div>
        ) : (
          <div className="history-list">
            <div className="history-table-head" aria-hidden="true">
              <span>Состояние и дата</span>
              <span>Результат</span>
              <span>Действия</span>
            </div>
            {entries.map((entry) => {
              const busy = busySession === entry.sessionId;
              return (
                <article className="history-item" key={entry.sessionId}>
                  <header className="history-item-heading">
                    <div>
                      <span className={"history-status status-" + entry.status}>
                        {entry.recoveryRequired ? (
                          <WarningCircle
                            aria-hidden="true"
                            size={17}
                            weight="regular"
                          />
                        ) : (
                          <CheckCircle
                            aria-hidden="true"
                            size={17}
                            weight="regular"
                          />
                        )}
                        {recoveryLabel(entry.status)}
                      </span>
                      <time dateTime={new Date(entry.updatedAt).toISOString()}>
                        {new Date(entry.updatedAt).toLocaleString("ru-RU")}
                      </time>
                    </div>
                    {entry.pinned ? (
                      <span className="pin-label">
                        <PushPin aria-hidden="true" size={14} weight="fill" />
                        Закреплено
                      </span>
                    ) : null}
                  </header>

                  {entry.selected ? (
                    <p className="transcript">{entry.selected.content}</p>
                  ) : (
                    <p className="transcript transcript-missing">
                      Текст ещё недоступен. Аудио сохранено для восстановления.
                    </p>
                  )}

                  {entry.cleaned && entry.raw ? (
                    <details className="history-details">
                      <summary>Показать исходный текст</summary>
                      <p className="transcript transcript-raw">
                        {entry.raw.content}
                      </p>
                    </details>
                  ) : null}

                  {entry.operations.length > 0 ? (
                    <details className="history-details">
                      <summary>
                        Технические доказательства доставки ·{" "}
                        {entry.operations.length}
                      </summary>
                      <ol className="attempt-list">
                        {entry.operations.map((operation) => (
                          <li key={operation.operationId}>
                            <strong>
                              #{operation.operationNo} {operation.status}
                            </strong>
                            <span>
                              {operation.initiatedBy} ·{" "}
                              {operation.confirmationLevel}
                            </span>
                            {operation.attempts.map((attempt) => (
                              <span key={attempt.attemptId}>
                                {attempt.ordinal}. {attempt.method} ·{" "}
                                {attempt.evidenceClass}
                                {attempt.errorCode
                                  ? " · " + attempt.errorCode
                                  : ""}
                              </span>
                            ))}
                          </li>
                        ))}
                      </ol>
                    </details>
                  ) : null}

                  <div className="history-actions">
                    {canRetry(entry) ? (
                      <button
                        aria-label="Вставить снова"
                        className="button button-warning button-icon"
                        disabled={busy}
                        onClick={() => onRetry(entry)}
                        title="Вставить снова"
                        type="button"
                      >
                        <ArrowCounterClockwise
                          aria-hidden="true"
                          size={18}
                          weight="regular"
                        />
                      </button>
                    ) : null}
                    {entry.selected ? (
                      <button
                        aria-label="Копировать"
                        className="button button-ghost button-icon"
                        disabled={busy}
                        onClick={() => onCopy(entry)}
                        title="Копировать"
                        type="button"
                      >
                        <Copy aria-hidden="true" size={18} weight="regular" />
                      </button>
                    ) : null}
                    {entry.recoveryRequired ? (
                      <button
                        aria-label="Считать решённым"
                        className="button button-ghost button-icon"
                        disabled={busy}
                        onClick={() => onResolve(entry)}
                        title="Считать решённым"
                        type="button"
                      >
                        <CheckCircle
                          aria-hidden="true"
                          size={18}
                          weight="regular"
                        />
                      </button>
                    ) : null}
                    <button
                      aria-label={entry.pinned ? "Открепить" : "Закрепить"}
                      className="button button-ghost button-icon"
                      disabled={busy}
                      onClick={() => onSetPinned(entry)}
                      title={entry.pinned ? "Открепить" : "Закрепить"}
                      type="button"
                    >
                      {entry.pinned ? (
                        <PushPinSlash
                          aria-hidden="true"
                          size={18}
                          weight="regular"
                        />
                      ) : (
                        <PushPin
                          aria-hidden="true"
                          size={18}
                          weight="regular"
                        />
                      )}
                    </button>
                    {canDelete(entry) ? (
                      <button
                        aria-label="Удалить"
                        className="button button-ghost button-danger button-icon"
                        disabled={busy}
                        onClick={() => onDelete(entry)}
                        title="Удалить"
                        type="button"
                      >
                        <Trash aria-hidden="true" size={18} weight="regular" />
                      </button>
                    ) : null}
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}

function SettingsSection({
  value,
  error,
  saving,
  onReload,
  onSave,
}: {
  value: SettingsView | null;
  error: string | null;
  saving: boolean;
  onReload: () => void;
  onSave: (update: ConfigurationUpdate) => void;
}) {
  const [draft, setDraft] = useState<ConfigurationUpdate | null>(
    value ? configurationUpdate(value.configuration) : null,
  );
  const [validationError, setValidationError] = useState<string | null>(null);
  const [pickingDirectory, setPickingDirectory] = useState(false);

  useEffect(() => {
    setDraft(value ? configurationUpdate(value.configuration) : null);
  }, [value]);

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!draft) return;
    const validation = validateConfiguration(draft);
    setValidationError(validation);
    if (!validation)
      onSave({
        ...draft,
        hotkeyBinding: draft.hotkeyBinding.trim(),
        archiveDirectory: draft.archiveDirectory.trim(),
      });
  }

  async function pickArchiveDirectory() {
    setPickingDirectory(true);
    setValidationError(null);
    try {
      const selected = await invoke<string | null>("archive_directory_pick");
      if (selected && draft) {
        setDraft({ ...draft, archiveDirectory: selected });
      }
    } catch (reason: unknown) {
      setValidationError(String(reason));
    } finally {
      setPickingDirectory(false);
    }
  }

  if (!value || !draft) {
    return (
      <section className="section-card">
        <div className="section-heading">
          <div>
            <p className="section-label">Versioned snapshot</p>
            <h2>Настройки недоступны</h2>
          </div>
          <button className="button" onClick={onReload} type="button">
            Повторить
          </button>
        </div>
        {error ? <ErrorNotice detail={error} /> : null}
      </section>
    );
  }

  return (
    <form className="settings-form" onSubmit={submit}>
      <section className="section-card settings-group">
        <div className="section-heading">
          <div>
            <p className="section-label">Управление диктовкой</p>
            <h2>Ввод и обработка</h2>
          </div>
          <span className="snapshot-label">
            snapshot {value.configuration.configVersion}
          </span>
        </div>

        <label className="field">
          <span>Горячая клавиша</span>
          <input
            autoComplete="off"
            onChange={(event) =>
              setDraft({ ...draft, hotkeyBinding: event.target.value })
            }
            spellCheck={false}
            value={draft.hotkeyBinding}
          />
          <small>
            Например: F8 или control+alt+Space. Escape зарезервирован.
          </small>
        </label>

        <div className="field-grid">
          <label className="field">
            <span>Микрофон</span>
            <select
              onChange={(event) =>
                setDraft({
                  ...draft,
                  microphoneDeviceId: event.target.value || null,
                })
              }
              value={draft.microphoneDeviceId ?? ""}
            >
              <option value="">Системный по умолчанию</option>
              {value.inputDevices.map((device) => (
                <option
                  disabled={!device.healthy}
                  key={device.id}
                  value={device.id}
                >
                  {device.name}
                  {device.isDefault ? " · по умолчанию" : ""}
                  {!device.healthy ? " · недоступен" : ""}
                </option>
              ))}
            </select>
          </label>

          <label className="field">
            <span>Локальная модель</span>
            <select
              onChange={(event) => {
                const activeRuntimeProfileId = event.target.value || null;
                setDraft({
                  ...draft,
                  activeRuntimeProfileId,
                  warmupEnabled: activeRuntimeProfileId
                    ? draft.warmupEnabled
                    : false,
                });
              }}
              value={draft.activeRuntimeProfileId ?? ""}
            >
              <option value="">Модель не выбрана</option>
              {value.runtimeProfiles.map((profile) => (
                <option
                  disabled={!profile.available}
                  key={profile.id}
                  value={profile.id}
                >
                  {runtimeLabel(profile)}
                  {!profile.available ? " · недоступна" : ""}
                </option>
              ))}
            </select>
          </label>
        </div>

        {value.inputDeviceError ? (
          <div className="device-warning" role="status">
            <strong>Не удалось проверить микрофоны.</strong>
            <span>Проверьте доступ приложения к микрофону в Windows.</span>
            <details>
              <summary>Технические детали</summary>
              <code>{value.inputDeviceError}</code>
            </details>
          </div>
        ) : null}

        <label className="field">
          <span>Очистка текста</span>
          <select
            onChange={(event) =>
              setDraft({
                ...draft,
                activeCleanupProfileId: event.target.value || null,
              })
            }
            value={draft.activeCleanupProfileId ?? ""}
          >
            <option value="">Консервативная встроенная</option>
            {value.cleanupProfiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.name} · v{profile.profileVersion}
              </option>
            ))}
          </select>
          <small>Исходный текст всегда сохраняется отдельно.</small>
        </label>
      </section>

      <section className="section-card settings-group">
        <div className="section-heading">
          <div>
            <p className="section-label">Локальные файлы</p>
            <h2>Архив записей</h2>
          </div>
        </div>
        <div className="archive-picker">
          <label className="field">
            <span>Папка для аудио и расшифровок</span>
            <input
              autoComplete="off"
              onChange={(event) =>
                setDraft({ ...draft, archiveDirectory: event.target.value })
              }
              spellCheck={false}
              value={draft.archiveDirectory}
            />
          </label>
          <button
            className="button button-ghost archive-picker-button"
            disabled={pickingDirectory}
            onClick={() => void pickArchiveDirectory()}
            type="button"
          >
            <FolderOpen
              className="button-leading-icon"
              aria-hidden="true"
              size={18}
              weight="regular"
            />
            {pickingDirectory ? "Выбор…" : "Выбрать папку"}
          </button>
        </div>
        <p className="archive-hint">
          Для каждой диктовки сохраняются обычные WAV и TXT с одинаковым именем.
          При смене папки доступная история копируется туда, а внутреннее
          recovery-хранилище остаётся страховкой.
        </p>
      </section>

      <section className="section-card settings-group">
        <div className="section-heading">
          <div>
            <p className="section-label">Поведение оболочки</p>
            <h2>Запуск и локальная диагностика</h2>
          </div>
        </div>
        <ToggleField
          checked={draft.startupEnabled}
          description="Обычный user-level запуск без повышения прав."
          label="Запускать с Windows"
          onChange={(checked) =>
            setDraft({ ...draft, startupEnabled: checked })
          }
          status={
            value.startupRegistered
              ? "Startup entry зарегистрирован"
              : "Startup entry отсутствует"
          }
        />
        <ToggleField
          checked={draft.warmupEnabled}
          description="Подготавливать выбранный локальный runtime после запуска."
          disabled={!draft.activeRuntimeProfileId}
          label="Прогревать модель"
          onChange={(checked) => setDraft({ ...draft, warmupEnabled: checked })}
        />
        <ToggleField
          checked={draft.diagnosticMode}
          description="Больше технических событий без текста и аудио. Экспорт не выполняется автоматически."
          label="Расширенная локальная диагностика"
          onChange={(checked) =>
            setDraft({ ...draft, diagnosticMode: checked })
          }
        />
      </section>

      <DiagnosticBundlePanel />
      {validationError ? (
        <p className="form-error" role="alert">
          {validationError}
        </p>
      ) : null}
      {error ? <ErrorNotice detail={error} /> : null}
      <div className="form-actions">
        <button
          className="button button-primary"
          disabled={saving}
          type="submit"
        >
          {saving ? "Сохранение…" : "Сохранить настройки"}
        </button>
        <button
          className="button button-ghost"
          onClick={onReload}
          type="button"
        >
          Отменить изменения
        </button>
      </div>
    </form>
  );
}

function ToggleField({
  checked,
  description,
  disabled = false,
  label,
  onChange,
  status,
}: {
  checked: boolean;
  description: string;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
  status?: string;
}) {
  return (
    <label className={"toggle-field" + (disabled ? " is-disabled" : "")}>
      <span>
        <strong>{label}</strong>
        <small>{description}</small>
        {status ? <em>{status}</em> : null}
      </span>
      <input
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
        type="checkbox"
      />
      <i aria-hidden="true" />
    </label>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function DiagnosticBundlePanel() {
  const [view, setView] = useState<DiagnosticView | null>(null);
  const [preview, setPreview] = useState<DiagnosticBundlePreview | null>(null);
  const [destination, setDestination] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState<"status" | "preview" | "export" | null>(
    "status",
  );
  const [error, setError] = useState<string | null>(null);
  const [receipt, setReceipt] = useState<DiagnosticExportReceipt | null>(null);

  useEffect(() => {
    let disposed = false;
    void invoke<DiagnosticView>("diagnostic_status")
      .then((status) => {
        if (!disposed) setView(status);
      })
      .catch((reason: unknown) => {
        if (!disposed) setError(String(reason));
      })
      .finally(() => {
        if (!disposed) setBusy(null);
      });
    return () => {
      disposed = true;
    };
  }, []);

  async function createPreview() {
    setBusy("preview");
    setError(null);
    setReceipt(null);
    setConfirmed(false);
    try {
      setPreview(await invoke<DiagnosticBundlePreview>("diagnostic_prepare"));
      setView(await invoke<DiagnosticView>("diagnostic_status"));
    } catch (reason: unknown) {
      setPreview(null);
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }

  async function exportBundle() {
    if (!preview) return;
    const validation = validateDiagnosticDestination(destination.trim());
    if (validation) {
      setError(validation);
      return;
    }
    if (!confirmed) {
      setError("Подтвердите экспорт после просмотра состава.");
      return;
    }
    setBusy("export");
    setError(null);
    try {
      const exported = await invoke<DiagnosticExportReceipt>(
        "diagnostic_export",
        {
          request: {
            previewId: preview.previewId,
            destinationPath: destination.trim(),
            confirmation: DIAGNOSTIC_EXPORT_CONFIRMATION,
          },
        },
      );
      setReceipt(exported);
      setPreview(null);
      setConfirmed(false);
      setDestination("");
      setView(await invoke<DiagnosticView>("diagnostic_status"));
    } catch (reason: unknown) {
      setError(String(reason));
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="section-card settings-group diagnostic-panel">
      <div className="section-heading">
        <div>
          <p className="section-label">Content-free support trace</p>
          <h2>Диагностический пакет</h2>
        </div>
        <span className="snapshot-label">
          schema {view?.traceSchemaVersion ?? "—"}
        </span>
      </div>
      <p className="diagnostic-intro">
        Пакет создаётся только по этой кнопке. Текст диктовки, аудио, буфер
        обмена, заголовки окон, абсолютные пути, окружение и секреты исключены
        схемой.
      </p>
      <div className="diagnostic-stats" aria-label="Состояние диагностики">
        <Metric label="Событий" value={view ? String(view.eventCount) : "—"} />
        <Metric
          label="На диске"
          value={view ? formatDiagnosticBytes(view.storedBytes) : "—"}
        />
        <Metric
          label="Хранение"
          value={view ? view.retentionDays + " дней" : "—"}
        />
        <Metric
          label="Лимит"
          value={view ? formatDiagnosticBytes(view.maximumBytes) : "—"}
        />
      </div>
      <div className="diagnostic-actions">
        <button
          className="button"
          disabled={busy !== null}
          onClick={() => void createPreview()}
          type="button"
        >
          {busy === "preview" ? "Проверка…" : "Сформировать предпросмотр"}
        </button>
        <span>
          Расширенные события:{" "}
          {view?.expandedEventsEnabled ? "включены" : "выключены"}
        </span>
      </div>
      {preview ? (
        <div className="bundle-preview" aria-live="polite">
          <div className="bundle-preview-title">
            <div>
              <strong>Состав готов к проверке</strong>
              <span>
                {preview.eventCount} событий ·{" "}
                {formatDiagnosticBytes(preview.byteCount)} · bundle schema{" "}
                {preview.bundleSchemaVersion}
              </span>
            </div>
            <span>{preview.sourceFileCount} файлов trace</span>
          </div>
          <div className="exclusion-list" aria-label="Исключено из пакета">
            {preview.excludedByDefault.map((entry) => (
              <span key={entry}>{diagnosticExclusionLabel(entry)}</span>
            ))}
          </div>
          <label className="field">
            <span>Полный путь сохранения</span>
            <input
              autoComplete="off"
              onChange={(event) => {
                setDestination(event.target.value);
                setConfirmed(false);
              }}
              placeholder="C:\Users\…\WiGigaDict-support.wigigadiag.json"
              spellCheck={false}
              value={destination}
            />
            <small>Существующий файл не будет перезаписан.</small>
          </label>
          <label className="diagnostic-confirm">
            <input
              checked={confirmed}
              onChange={(event) => setConfirmed(event.target.checked)}
              type="checkbox"
            />
            <span>
              Я просмотрел состав и явно подтверждаю локальный экспорт.
            </span>
          </label>
          <button
            className="button button-primary"
            disabled={!confirmed || busy !== null}
            onClick={() => void exportBundle()}
            type="button"
          >
            {busy === "export" ? "Экспорт…" : "Экспортировать пакет"}
          </button>
        </div>
      ) : null}
      {receipt ? (
        <p className="diagnostic-receipt" role="status">
          Сохранён файл {receipt.fileName}: {receipt.eventCount} событий,{" "}
          {formatDiagnosticBytes(receipt.byteCount)}.
        </p>
      ) : null}
      {error ? (
        <p className="form-error" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}

function diagnosticExclusionLabel(value: string): string {
  const labels: Record<string, string> = {
    audio: "аудио",
    transcript: "текст",
    clipboard: "буфер обмена",
    window_title: "заголовки окон",
    absolute_path: "абсолютные пути",
    environment: "окружение",
    secret: "секреты",
    token: "токены",
  };
  return labels[value] ?? "неизвестное поле";
}

function ConfirmDialog({
  confirmation,
  onCancel,
  onConfirm,
}: {
  confirmation: Confirmation;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const confirmButton = useRef<HTMLButtonElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const retry = confirmation.kind === "retry";

  useEffect(() => {
    confirmButton.current?.focus();
  }, []);

  function handleKeyDown(event: React.KeyboardEvent) {
    if (event.key === "Escape") {
      event.stopPropagation();
      onCancel();
      return;
    }
    if (event.key !== "Tab") return;
    const active = document.activeElement;
    const current: DialogControl | null =
      active === confirmButton.current
        ? "confirm"
        : active === cancelButton.current
          ? "cancel"
          : null;
    if (!current) return;
    const next = nextDialogControl(current, event.shiftKey);
    if (next) {
      event.preventDefault();
      (next === "confirm" ? confirmButton : cancelButton).current?.focus();
    }
  }

  return (
    <div className="dialog-backdrop" onKeyDown={handleKeyDown}>
      <section
        aria-describedby="dialog-description"
        aria-labelledby="dialog-title"
        aria-modal="true"
        className="confirm-dialog"
        role="alertdialog"
      >
        <span className={"dialog-signal " + (retry ? "warning" : "danger")} />
        <p className="section-label">
          {retry ? "Риск дубликата" : "Необратимое действие"}
        </p>
        <h2 id="dialog-title">
          {retry ? "Вставить текст снова?" : "Удалить локальный результат?"}
        </h2>
        <p id="dialog-description">
          {retry
            ? "Повторная вставка использует поле, активное сейчас. Предыдущая доставка могла состояться, поэтому проверьте поле на дубликат."
            : "Будут удалены текст, попытки доставки и все управляемые копии аудио. Восстановить их после удаления нельзя."}
        </p>
        <div className="dialog-actions">
          <button
            className={"button " + (retry ? "button-warning" : "button-danger")}
            onClick={onConfirm}
            ref={confirmButton}
            type="button"
          >
            {retry ? "Вставить снова" : "Удалить"}
          </button>
          <button
            className="button button-ghost"
            onClick={onCancel}
            ref={cancelButton}
            type="button"
          >
            Отмена
          </button>
        </div>
      </section>
    </div>
  );
}

function ErrorNotice({ detail }: { detail: string }) {
  return (
    <div className="error-notice" role="alert">
      <strong>Изменение не применено.</strong>
      <span>
        Текущие данные сохранены. Обновите состояние и повторите действие.
      </span>
      <details>
        <summary>Технические детали</summary>
        <code>{detail}</code>
      </details>
    </div>
  );
}

function friendlyReason(reason: string): ReactNode {
  const reasons: Record<string, string> = {
    audio_device_lost: "Микрофон отключён. Доступная запись сохранена.",
    cancelled: "Запись отменена.",
    empty_capture: "Речь не записана. Проверьте микрофон.",
    startup_reconciliation_failed: "Незавершённая сессия доступна в recovery.",
  };
  return reasons[reason] ?? "Проверьте настройки и готовность локальных служб.";
}

export default App;
