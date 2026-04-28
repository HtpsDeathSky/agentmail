import type { SendMessageDraft, SendMessageResult, SyncSummary } from "../api";
import { formatSendStatus } from "./format";

export type MailSyncEventPayload = {
  account_id: string;
  folder_id?: string | null;
  reason: string;
  message?: string | null;
};

export type DirectSendFlowRequest = {
  draft: SendMessageDraft;
  selectedFolderId: string | null;
  query: string;
  sendMessage: (draft: SendMessageDraft) => Promise<SendMessageResult>;
  refreshFolders: (accountId: string) => Promise<void>;
  refreshMessages: (accountId: string, folderId: string | null, query: string) => Promise<void>;
  refreshAudits: () => Promise<void>;
};

export type DirectSendFlowResult =
  | {
      ok: true;
      status: string;
    }
  | {
      ok: false;
      status: string;
      error: unknown;
    };

function firstRejectedReason(results: PromiseSettledResult<unknown>[]) {
  return results.find((result): result is PromiseRejectedResult => result.status === "rejected")?.reason;
}

export async function runDirectSendFlow({
  draft,
  selectedFolderId,
  query,
  sendMessage,
  refreshFolders,
  refreshMessages,
  refreshAudits
}: DirectSendFlowRequest): Promise<DirectSendFlowResult> {
  let sendResult: SendMessageResult;
  try {
    sendResult = await sendMessage(draft);
  } catch (error) {
    await Promise.allSettled([refreshAudits()]);
    return {
      ok: false,
      status: `send failed: ${String(error)}`,
      error
    };
  }

  const refreshResults = await Promise.allSettled([
    refreshFolders(draft.account_id),
    refreshMessages(draft.account_id, selectedFolderId, query),
    refreshAudits()
  ]);
  const sentStatus = formatSendStatus(draft.to);
  const statusParts = [sentStatus];
  if (sendResult.warning) statusParts.push(sendResult.warning);
  const refreshError = firstRejectedReason(refreshResults);
  if (refreshError) statusParts.push(`refresh failed: ${String(refreshError)}`);
  return {
    ok: true,
    status: statusParts.join(" / ")
  };
}

export type InitialAccountSyncRequest = {
  accountId: string;
  email: string;
  syncEnabled?: boolean;
  folderId: string | null;
  query: string;
  syncAccount: (accountId: string) => Promise<SyncSummary>;
  startAccountWatchers?: (accountId: string) => Promise<unknown>;
  refreshFolders: (accountId: string) => Promise<void>;
  refreshMessages: (accountId: string, folderId: string | null, query: string) => Promise<void>;
  refreshSyncState: (accountId: string) => Promise<void>;
  refreshAudits: () => Promise<void>;
};

export async function runInitialAccountSync({
  accountId,
  email,
  syncEnabled = true,
  folderId,
  query,
  syncAccount,
  startAccountWatchers,
  refreshFolders,
  refreshMessages,
  refreshSyncState,
  refreshAudits
}: InitialAccountSyncRequest) {
  if (!syncEnabled) {
    await Promise.allSettled([
      refreshFolders(accountId),
      refreshMessages(accountId, folderId, query),
      refreshSyncState(accountId),
      refreshAudits()
    ]);
    return `account configuration saved: ${email}`;
  }

  let summary: SyncSummary | null = null;
  let syncError: unknown = null;

  try {
    summary = await syncAccount(accountId);
  } catch (error) {
    syncError = error;
  }

  if (summary && startAccountWatchers) {
    await startAccountWatchers(accountId).catch(() => undefined);
  }

  await Promise.allSettled([
    refreshFolders(accountId),
    refreshMessages(accountId, folderId, query),
    refreshSyncState(accountId),
    refreshAudits()
  ]);

  if (summary) {
    return `account saved and initial sync complete: ${email} / ${summary.folders} folders / ${summary.messages} messages`;
  }
  return `account saved, but initial sync failed: ${String(syncError)}`;
}

export type RefreshAfterMailSyncEventRequest = {
  payload: MailSyncEventPayload;
  selectedAccountId: string | null;
  selectedFolderId: string | null;
  query: string;
  refreshFolders: (accountId: string) => Promise<void>;
  refreshMessages: (accountId: string, folderId: string | null, query: string) => Promise<void>;
  refreshSyncState: (accountId: string) => Promise<void>;
  refreshAudits: () => Promise<void>;
};

export async function refreshAfterMailSyncEvent({
  payload,
  selectedAccountId,
  selectedFolderId,
  query,
  refreshFolders,
  refreshMessages,
  refreshSyncState,
  refreshAudits
}: RefreshAfterMailSyncEventRequest) {
  if (!selectedAccountId || payload.account_id !== selectedAccountId) {
    return false;
  }

  await Promise.allSettled([
    refreshFolders(payload.account_id),
    refreshMessages(payload.account_id, selectedFolderId, query),
    refreshSyncState(payload.account_id),
    refreshAudits()
  ]);
  return true;
}

export type ManualAccountSyncRequest = {
  accountId: string;
  folderId: string | null;
  query: string;
  syncAccount: (accountId: string) => Promise<SyncSummary>;
  startAccountWatchers?: (accountId: string) => Promise<unknown>;
  refreshFolders: (accountId: string) => Promise<void>;
  refreshMessages: (accountId: string, folderId: string | null, query: string) => Promise<void>;
  refreshSyncState: (accountId: string) => Promise<void>;
  refreshAudits: () => Promise<void>;
};

export async function runManualAccountSync({
  accountId,
  folderId,
  query,
  syncAccount,
  startAccountWatchers,
  refreshFolders,
  refreshMessages,
  refreshSyncState,
  refreshAudits
}: ManualAccountSyncRequest) {
  const summary = await syncAccount(accountId);
  if (startAccountWatchers) {
    await startAccountWatchers(accountId).catch(() => undefined);
  }
  await Promise.allSettled([
    refreshFolders(accountId),
    refreshMessages(accountId, folderId, query),
    refreshSyncState(accountId),
    refreshAudits()
  ]);
  return `sync complete: ${summary.folders} folders / ${summary.messages} messages`;
}
