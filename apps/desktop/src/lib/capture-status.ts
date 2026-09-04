export type CapturePhase =
  | "idle"
  | "preparing"
  | "recording"
  | "finalizing"
  | "recovery"
  | "unavailable";

export type CaptureStatus = {
  phase: CapturePhase;
  sessionId: string | null;
  reason: string | null;
  deviceHealthy: boolean;
  durablePcmBytes: number;
  /// Times the audio device dropped samples during this capture.
  audioGaps: number;
};

export const initialCaptureStatus: CaptureStatus = {
  phase: "unavailable",
  sessionId: null,
  reason: "Ожидание локальной службы записи",
  deviceHealthy: false,
  durablePcmBytes: 0,
  audioGaps: 0,
};

const labels: Record<CapturePhase, string> = {
  idle: "Диктовка готова",
  preparing: "Подготовка записи…",
  recording: "Запись",
  finalizing: "Сохранение записи…",
  recovery: "Запись сохранена в recovery",
  unavailable: "Диктовка недоступна",
};

export function captureLabel(phase: CapturePhase): string {
  return labels[phase];
}

/// A survived xrun keeps the recording alive but the dropped samples are gone, so the sentence
/// may come back with a hole in it. Saying so plainly beats letting the user wonder why a word
/// went missing.
export function audioGapWarning(status: CaptureStatus): string | null {
  if (status.audioGaps <= 0) return null;
  const times =
    status.audioGaps === 1 ? "один раз" : `${status.audioGaps} раза`;
  return `Звуковое устройство ${times} не успело за записью — часть звука потеряна, в тексте может не хватать слов.`;
}

export function canCancelCapture(phase: CapturePhase): boolean {
  return phase === "preparing" || phase === "recording";
}
