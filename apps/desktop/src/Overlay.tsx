import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Check,
  Sparkle,
  SpinnerGap,
  WarningCircle,
  X,
} from "@phosphor-icons/react";

import {
  overlayCompactLabel,
  overlayCopy,
  overlayWaveformAmplitudes,
  overlayWaveformScale,
  type OverlayPhase,
  type OverlayStatus,
} from "./lib/overlay-status";

const development = (import.meta as ImportMeta & { env: { DEV: boolean } }).env
  .DEV;
const previewParameters = development
  ? new URLSearchParams(window.location.search)
  : null;
const previewPhase = (
  [
    "preparing",
    "recording",
    "processing",
    "delivered",
    "uncertain",
    "error",
  ] as const
).find((phase) => previewParameters?.get("phase") === phase);
const previewCycle = previewParameters?.get("preview") === "cycle";

const initialOverlay: OverlayStatus = {
  phase: previewCycle ? "recording" : (previewPhase ?? "preparing"),
  sessionId: null,
  reason: null,
};

type CaptureLevel = {
  sessionId: string;
  level: number;
};

function StatusIcon({ phase }: { phase: OverlayPhase }) {
  if (phase === "preparing") {
    return <SpinnerGap weight="bold" aria-hidden="true" />;
  }
  if (phase === "recording") {
    return null;
  }
  if (phase === "processing") {
    return <Sparkle weight="fill" aria-hidden="true" />;
  }
  if (phase === "delivered") {
    return <Check weight="bold" aria-hidden="true" />;
  }
  return <WarningCircle weight="bold" aria-hidden="true" />;
}

export default function Overlay() {
  const [status, setStatus] = useState<OverlayStatus>(initialOverlay);
  const [actionPending, setActionPending] = useState(false);
  const [audioLevel, setAudioLevel] = useState(
    previewPhase === "recording" ? 0.58 : 0.04,
  );
  const statusRef = useRef(status);
  const copy = overlayCopy(status);
  const canFinish =
    status.phase === "preparing" || status.phase === "recording";
  const canCancel = canFinish || status.phase === "processing";

  useEffect(() => {
    statusRef.current = status;
    if (status.phase !== "recording") setAudioLevel(0.04);
  }, [status]);

  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let stopListening: (() => void) | undefined;

    void Promise.all([
      listen<OverlayStatus>("overlay-status", (event) => {
        if (!disposed) {
          setStatus(event.payload);
          setActionPending(false);
        }
      }),
      listen<CaptureLevel>("capture-level", (event) => {
        const active = statusRef.current;
        if (
          !disposed &&
          active.phase === "recording" &&
          active.sessionId === event.payload.sessionId
        ) {
          setAudioLevel(Math.max(0.04, Math.min(1, event.payload.level)));
        }
      }),
    ]).then((unlisten) => {
      const stopAll = () => unlisten.forEach((stop) => stop());
      if (disposed) stopAll();
      else stopListening = stopAll;
    });

    return () => {
      disposed = true;
      stopListening?.();
    };
  }, []);

  useEffect(() => {
    if (!previewCycle) return;
    const started = performance.now();
    const timer = window.setInterval(() => {
      const elapsed = (performance.now() - started) % 3000;
      const recording = elapsed < 1800;
      setStatus((current) => {
        const phase = recording ? "recording" : "processing";
        return current.phase === phase
          ? current
          : { phase, sessionId: "dev-overlay-preview", reason: null };
      });
      if (recording) {
        const wave = Math.abs(Math.sin((elapsed / 1800) * Math.PI * 4));
        setAudioLevel(0.06 + wave * 0.66);
      }
    }, 66);
    return () => window.clearInterval(timer);
  }, []);

  function request(
    command: "overlay_cancel_capture" | "overlay_finish_capture",
  ) {
    const actionAllowed =
      command === "overlay_cancel_capture" ? canCancel : canFinish;
    if (
      !isTauri() ||
      previewCycle ||
      previewPhase ||
      !actionAllowed ||
      actionPending
    )
      return;
    setActionPending(true);
    void invoke(command).catch(() => setActionPending(false));
  }

  return (
    <main
      className={`overlay-card overlay-${status.phase}`}
      role="status"
      aria-label={`${copy.title}. ${copy.detail}`}
      aria-live="assertive"
      aria-atomic="true"
    >
      <button
        className="overlay-endcap overlay-cancel"
        type="button"
        tabIndex={-1}
        aria-label="Отменить запись"
        disabled={!canCancel || actionPending}
        onClick={() => request("overlay_cancel_capture")}
      >
        <X weight="bold" aria-hidden="true" />
      </button>
      {status.phase === "recording" ? (
        <div className="overlay-live-meter" aria-label="Уровень микрофона">
          {overlayWaveformAmplitudes.map((amplitude, index) => (
            <span
              key={index}
              style={
                {
                  "--bar-scale": overlayWaveformScale(audioLevel, amplitude),
                } as CSSProperties
              }
            />
          ))}
        </div>
      ) : (
        <div className="overlay-body">
          <span className="overlay-label">{overlayCompactLabel(status)}</span>
          <span className="overlay-state-icon">
            <StatusIcon phase={status.phase} />
          </span>
        </div>
      )}
      {canFinish ? (
        <button
          className="overlay-endcap overlay-result"
          type="button"
          tabIndex={-1}
          aria-label="Завершить запись"
          disabled={actionPending}
          onClick={() => request("overlay_finish_capture")}
        >
          <Check weight="bold" aria-hidden="true" />
        </button>
      ) : (
        <span className="overlay-slot" aria-hidden="true" />
      )}
    </main>
  );
}
