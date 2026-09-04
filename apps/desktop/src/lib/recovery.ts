export type RecoveryStatus =
  "pending" | "delivered" | "uncertain" | "copied" | "resolved" | "cancelled";

export type RecoveryTranscript = {
  transcriptId: string;
  sessionId: string;
  kind: "raw" | "cleaned";
  content: string;
  contentHash: string;
  createdAt: number;
};

export type RecoveryAttempt = {
  attemptId: string;
  ordinal: number;
  method: string;
  status: string;
  evidenceClass: string;
  errorCode: string | null;
  startedAt: number;
  completedAt: number;
};

export type RecoveryOperation = {
  operationId: string;
  operationNo: number;
  initiatedBy: "system" | "user";
  userActionId: string | null;
  status: string;
  confirmationLevel: string;
  finalErrorCode: string | null;
  startedAt: number;
  completedAt: number | null;
  attempts: RecoveryAttempt[];
};

export type RecoveryEntry = {
  sessionId: string;
  pipelineState: string;
  stateVersion: number;
  status: RecoveryStatus;
  recoveryRequired: boolean;
  raw: RecoveryTranscript | null;
  cleaned: RecoveryTranscript | null;
  selected: RecoveryTranscript | null;
  operations: RecoveryOperation[];
  startedAt: number;
  updatedAt: number;
  deliveredAt: number | null;
  resolvedAt: number | null;
  pinned: boolean;
  retentionExpiresAt: number | null;
  lastErrorCode: string | null;
};

const labels: Record<RecoveryStatus, string> = {
  pending: "В обработке",
  delivered: "Доставлено",
  uncertain: "Вставка не подтверждена",
  copied: "Скопировано, не решено",
  resolved: "Решено",
  cancelled: "Отменено",
};

const activePipelineStates = new Set([
  "recording",
  "finalizing",
  "processing",
  "ready_to_deliver",
  "delivering",
]);

export function recoveryLabel(status: RecoveryStatus): string {
  return labels[status];
}

export function canRetry(entry: RecoveryEntry): boolean {
  return entry.recoveryRequired && entry.selected !== null;
}

export function canDelete(entry: RecoveryEntry): boolean {
  return !activePipelineStates.has(entry.pipelineState);
}
