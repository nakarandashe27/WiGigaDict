export type OverlayPhase =
  | "preparing"
  | "recording"
  | "processing"
  | "delivered"
  | "uncertain"
  | "error";

export type OverlayStatus = {
  phase: OverlayPhase;
  sessionId: string | null;
  reason: string | null;
};

type OverlayCopy = {
  title: string;
  detail: string;
};

const phaseCopy: Record<OverlayPhase, OverlayCopy> = {
  preparing: {
    title: "Подготовка записи",
    detail: "Проверяем локальное хранилище",
  },
  recording: {
    title: "Запись",
    detail: "Нажмите горячую клавишу ещё раз, чтобы завершить",
  },
  processing: {
    title: "Обработка локально",
    detail: "Результат сохраняется до попытки вставки",
  },
  delivered: {
    title: "Текст вставлен",
    detail: "Доставка подтверждена",
  },
  uncertain: {
    title: "Вставка не подтверждена",
    detail: "Текст сохранён в recovery",
  },
  error: {
    title: "Результат требует внимания",
    detail: "Откройте recovery в главном окне",
  },
};

const reasonCopy: Record<string, OverlayCopy> = {
  audio_device_lost: {
    title: "Микрофон отключён",
    detail: "Доступная часть записи сохранена в recovery",
  },
  audio_queue_overflow: {
    title: "Запись остановлена",
    detail: "Доступная часть сохранена в recovery",
  },
  empty_capture: {
    title: "Речь не записана",
    detail: "Проверьте выбранный микрофон",
  },
  delivery_failed: {
    title: "Не удалось вставить текст",
    detail: "Результат сохранён в recovery",
  },
  delivery_unconfirmed: {
    title: "Вставка не подтверждена",
    detail: "Текст сохранён в recovery",
  },
  delivery_transport_only: {
    title: "Текст вставлен (без подтверждения)",
    detail: "Копия сохранена в recovery",
  },
  empty_transcript: {
    title: "Распознан пустой текст",
    detail: "Вставлять нечего — проверьте запись",
  },
  processing_cpu_fallback: {
    title: "GPU занята — обработка на CPU",
    detail: "Распознавание может занять заметно дольше",
  },
  cleanup_failed: {
    title: "Очистка не применена",
    detail: "Исходный текст сохранён",
  },
  pcm_size_limit: {
    title: "Достигнут предел записи",
    detail: "Записанное распознаётся как обычно",
  },
  asr_unavailable: {
    title: "Распознавание недоступно",
    detail: "Запись сохранена — откройте восстановление",
  },
  runtime_contract_invalid: {
    title: "Модель недоступна",
    detail: "Аудио сохранено до восстановления сессии",
  },
  transcript_commit_failed: {
    title: "Текст не зафиксирован",
    detail: "Аудио сохранено до восстановления сессии",
  },
};

export function overlayCopy(status: OverlayStatus): OverlayCopy {
  return status.reason
    ? (reasonCopy[status.reason] ?? phaseCopy[status.phase])
    : phaseCopy[status.phase];
}

// The HUD pill has one invariant width, so every compact label must fit its centre column.
const compactLabels: Record<OverlayPhase, string> = {
  preparing: "Подготовка",
  recording: "Запись",
  processing: "Обработка",
  delivered: "Готово",
  uncertain: "Сохранено",
  error: "Проверьте",
};

export function overlayCompactLabel(status: OverlayStatus): string {
  return compactLabels[status.phase];
}

export function isTerminalOverlayPhase(phase: OverlayPhase): boolean {
  return phase === "delivered" || phase === "uncertain" || phase === "error";
}

export const overlayWaveformAmplitudes = [
  0.46, 0.76, 1, 0.62, 0.88, 0.56, 0.72,
] as const;

export function overlayWaveformScale(level: number, amplitude: number): number {
  const boundedLevel = Math.max(0, Math.min(1, level));
  const boundedAmplitude = Math.max(0, Math.min(1, amplitude));
  return 0.28 + boundedLevel * boundedAmplitude * 0.72;
}
