import {
  Archive,
  BellDot,
  CheckCheck,
  CircleAlert,
  Clock3,
  Database,
  Folder,
  Inbox,
  Mail,
  MailPlus,
  Moon,
  PanelRight,
  RefreshCcw,
  Search,
  Send,
  Settings,
  ShieldCheck,
  Star,
  Sun,
  TerminalSquare,
  Trash2,
  X
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import {
  CSSProperties,
  FormEvent,
  KeyboardEvent as ReactKeyboardEvent,
  PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useTransition
} from "react";
import {
  AccountConfigView,
  AiInsight,
  AiSettingsView,
  api,
  MailAccount,
  MailActionAudit,
  MailActionKind,
  MailFolder,
  MailMessage,
  SaveAccountConfigRequest,
  SaveAiSettingsRequest,
  SendMessageDraft,
  SyncState,
  SyncSummary
} from "./api";

const defaultAccountConfigForm: SaveAccountConfigRequest = {
  id: null,
  display_name: "",
  email: "",
  password: "",
  imap_host: "",
  imap_port: 993,
  imap_tls: true,
  smtp_host: "",
  smtp_port: 465,
  smtp_tls: true,
  sync_enabled: true
};

export const MAIL_SYNC_EVENT = "agentmail-mail-sync";
export const AUTO_SYNC_INTERVAL_MS = 30_000;
export const THEME_MODE_STORAGE_KEY = "agentmail-theme-mode";
export const WORKSPACE_SPLIT_STORAGE_KEY = "agentmail-workspace-split-percent";
export const ACTIVITY_LOG_STORAGE_KEY = "agentmail-show-activity-log";
export const DEFAULT_WORKSPACE_SPLIT_PERCENT = 45;
const WORKSPACE_LIST_MIN_WIDTH = 320;
const WORKSPACE_DETAIL_MIN_WIDTH = 420;
const WORKSPACE_DIVIDER_WIDTH = 8;

export type ThemeMode = "dark" | "light";

export type MailSyncEventPayload = {
  account_id: string;
  folder_id?: string | null;
  reason: string;
  message?: string | null;
};

const roleIcon = {
  inbox: Inbox,
  sent: Send,
  archive: Archive,
  trash: Trash2,
  drafts: MailPlus,
  junk: CircleAlert,
  custom: Folder
};

const actionLabels: Record<MailActionKind, string> = {
  mark_read: "READ",
  mark_unread: "UNREAD",
  star: "STAR",
  unstar: "UNSTAR",
  move: "MOVE",
  archive: "ARCHIVE",
  delete: "DELETE",
  permanent_delete: "PURGE",
  send: "SEND",
  forward: "FORWARD",
  batch_delete: "BATCH DELETE",
  batch_move: "BATCH MOVE"
};

export function formatFolderCount(folder: Pick<MailFolder, "unread_count" | "total_count">) {
  if (folder.unread_count > 0) return `${folder.unread_count}/${folder.total_count}`;
  return String(folder.total_count);
}

export function formatSendStatus(recipients: string[]) {
  return `sent to ${recipients.join(", ")}`;
}

type DirectSendFlowRequest = {
  draft: SendMessageDraft;
  selectedFolderId: string | null;
  query: string;
  sendMessage: (draft: SendMessageDraft) => Promise<unknown>;
  refreshFolders: (accountId: string) => Promise<void>;
  refreshMessages: (accountId: string, folderId: string | null, query: string) => Promise<void>;
  refreshAudits: () => Promise<void>;
};

type DirectSendFlowResult =
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
  try {
    await sendMessage(draft);
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
  const refreshError = firstRejectedReason(refreshResults);
  const sentStatus = formatSendStatus(draft.to);
  return {
    ok: true,
    status: refreshError ? `${sentStatus} / refresh failed: ${String(refreshError)}` : sentStatus
  };
}

export function formatAuditLine(audit: MailActionAudit) {
  const base = `[${formatTime(audit.created_at)}] ${actionLabels[audit.action] ?? audit.action}:${audit.status}`;
  return audit.error_message ? `${base} / ${audit.error_message}` : base;
}

export function readStoredThemeMode(storage: Pick<Storage, "getItem"> | null | undefined): ThemeMode {
  try {
    return storage?.getItem(THEME_MODE_STORAGE_KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

export function getNextThemeMode(mode: ThemeMode): ThemeMode {
  return mode === "dark" ? "light" : "dark";
}

export function readStoredActivityLogVisibility(storage: Pick<Storage, "getItem"> | null | undefined) {
  try {
    return storage?.getItem(ACTIVITY_LOG_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

export function writeStoredActivityLogVisibility(storage: Pick<Storage, "setItem"> | null | undefined, visible: boolean) {
  try {
    storage?.setItem(ACTIVITY_LOG_STORAGE_KEY, visible ? "true" : "false");
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}

export function getAppShellClassName(showActivityLog: boolean) {
  return showActivityLog ? "app-shell activity-log-visible" : "app-shell";
}

export function applyThemeModeToDocument(
  root: Pick<HTMLElement, "dataset" | "style">,
  storage: Pick<Storage, "setItem"> | null | undefined,
  mode: ThemeMode
) {
  root.dataset.theme = mode;
  root.style.colorScheme = mode;
  try {
    storage?.setItem(THEME_MODE_STORAGE_KEY, mode);
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}

export function clampWorkspaceSplitPercent(
  percent: number,
  containerWidth: number,
  minListWidth: number,
  minDetailWidth: number
) {
  return getWorkspaceSplitModel(percent, containerWidth, minListWidth, minDetailWidth).percent;
}

export function getWorkspaceSplitModel(
  percent: number,
  containerWidth: number | null | undefined,
  minListWidth: number,
  minDetailWidth: number
) {
  if (!Number.isFinite(percent)) {
    return { percent: DEFAULT_WORKSPACE_SPLIT_PERCENT, minPercent: 0, maxPercent: 100 };
  }
  if (!Number.isFinite(containerWidth) || !containerWidth || containerWidth < minListWidth + minDetailWidth) {
    return { percent: 50, minPercent: 50, maxPercent: 50 };
  }
  const minPercent = (minListWidth / containerWidth) * 100;
  const maxPercent = 100 - (minDetailWidth / containerWidth) * 100;
  const clampedPercent = Math.min(Math.max(percent, minPercent), maxPercent);
  return {
    percent: clampedPercent,
    minPercent,
    maxPercent
  };
}

export function readStoredWorkspaceSplitPercent(storage: Pick<Storage, "getItem"> | null | undefined) {
  try {
    const stored = storage?.getItem(WORKSPACE_SPLIT_STORAGE_KEY);
    if (!stored) return DEFAULT_WORKSPACE_SPLIT_PERCENT;
    const percent = Number(stored);
    return Number.isFinite(percent) && percent > 0 && percent < 100 ? percent : DEFAULT_WORKSPACE_SPLIT_PERCENT;
  } catch {
    return DEFAULT_WORKSPACE_SPLIT_PERCENT;
  }
}

function writeStoredWorkspaceSplitPercent(storage: Pick<Storage, "setItem"> | null | undefined, percent: number) {
  try {
    storage?.setItem(WORKSPACE_SPLIT_STORAGE_KEY, String(percent));
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}

type InitialAccountSyncRequest = {
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

type RefreshAfterMailSyncEventRequest = {
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

type AutomaticAccountSyncRequest = {
  selectedAccountId: string | null;
  selectedFolderId: string | null;
  query: string;
  syncEnabled?: boolean;
  syncAccount: (accountId: string) => Promise<SyncSummary>;
  startAccountWatchers?: (accountId: string) => Promise<unknown>;
  refreshFolders: (accountId: string) => Promise<void>;
  refreshMessages: (accountId: string, folderId: string | null, query: string) => Promise<void>;
  refreshSyncState: (accountId: string) => Promise<void>;
  refreshAudits: () => Promise<void>;
};

export async function runAutomaticAccountSync({
  selectedAccountId,
  selectedFolderId,
  query,
  syncEnabled = true,
  syncAccount,
  startAccountWatchers,
  refreshFolders,
  refreshMessages,
  refreshSyncState,
  refreshAudits
}: AutomaticAccountSyncRequest) {
  if (!selectedAccountId || !syncEnabled) {
    return { refreshed: false, status: null };
  }

  const summary = await syncAccount(selectedAccountId);
  if (startAccountWatchers) {
    await startAccountWatchers(selectedAccountId).catch(() => undefined);
  }
  await Promise.allSettled([
    refreshFolders(selectedAccountId),
    refreshMessages(selectedAccountId, selectedFolderId, query),
    refreshSyncState(selectedAccountId),
    refreshAudits()
  ]);
  return {
    refreshed: true,
    status: `auto sync complete: ${summary.folders} folders / ${summary.messages} messages`
  };
}

type ManualAccountSyncRequest = {
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

export function App() {
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [folders, setFolders] = useState<MailFolder[]>([]);
  const [messages, setMessages] = useState<MailMessage[]>([]);
  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(null);
  const [selectedFolderId, setSelectedFolderId] = useState<string | null>(null);
  const [selectedMessageId, setSelectedMessageId] = useState<string | null>(null);
  const [selectedMessage, setSelectedMessage] = useState<MailMessage | null>(null);
  const [syncStates, setSyncStates] = useState<SyncState[]>([]);
  const [audits, setAudits] = useState<MailActionAudit[]>([]);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState("backend link idle");
  const [isConfigOpen, setConfigOpen] = useState(false);
  const [isComposerOpen, setComposerOpen] = useState(false);
  const [aiSettings, setAiSettings] = useState<AiSettingsView | null>(null);
  const [aiInsights, setAiInsights] = useState<AiInsight[]>([]);
  const [isAnalyzing, setAnalyzing] = useState(false);
  const [isActionRunning, setActionRunning] = useState(false);
  const [aiStatus, setAiStatus] = useState("ai link idle");
  const [themeMode, setThemeMode] = useState<ThemeMode>(() =>
    typeof window === "undefined" ? "dark" : readStoredThemeMode(window.localStorage)
  );
  const [showActivityLog, setShowActivityLog] = useState(() =>
    typeof window === "undefined" ? false : readStoredActivityLogVisibility(window.localStorage)
  );
  const [workspaceSplitPercent, setWorkspaceSplitPercent] = useState(() =>
    typeof window === "undefined" ? DEFAULT_WORKSPACE_SPLIT_PERCENT : readStoredWorkspaceSplitPercent(window.localStorage)
  );
  const [workspaceAvailableWidth, setWorkspaceAvailableWidth] = useState<number | null>(null);
  const [isWorkspaceResizing, setWorkspaceResizing] = useState(false);
  const [isPending, startTransition] = useTransition();
  const mailWorkspaceRef = useRef<HTMLDivElement | null>(null);
  const messagesRef = useRef<MailMessage[]>([]);
  const selectedAccountIdRef = useRef<string | null>(null);
  const selectedAccountSyncEnabledRef = useRef(true);
  const selectedFolderIdRef = useRef<string | null>(null);
  const selectedMessageIdRef = useRef<string | null>(null);
  const isAutoSyncRunningRef = useRef(false);
  const queryRef = useRef("");

  useEffect(() => {
    if (typeof document === "undefined") return;
    applyThemeModeToDocument(document.documentElement, typeof window === "undefined" ? null : window.localStorage, themeMode);
  }, [themeMode]);

  useEffect(() => {
    writeStoredActivityLogVisibility(typeof window === "undefined" ? null : window.localStorage, showActivityLog);
  }, [showActivityLog]);

  const workspaceSplitModel = useMemo(
    () =>
      getWorkspaceSplitModel(
        workspaceSplitPercent,
        workspaceAvailableWidth,
        WORKSPACE_LIST_MIN_WIDTH,
        WORKSPACE_DETAIL_MIN_WIDTH
      ),
    [workspaceAvailableWidth, workspaceSplitPercent]
  );

  useEffect(() => {
    if (workspaceAvailableWidth === null) return;
    setWorkspaceSplitPercent((current) =>
      getWorkspaceSplitModel(current, workspaceAvailableWidth, WORKSPACE_LIST_MIN_WIDTH, WORKSPACE_DETAIL_MIN_WIDTH).percent
    );
  }, [workspaceAvailableWidth]);

  useEffect(() => {
    if (workspaceAvailableWidth === null) return;
    writeStoredWorkspaceSplitPercent(typeof window === "undefined" ? null : window.localStorage, workspaceSplitModel.percent);
  }, [workspaceAvailableWidth, workspaceSplitModel.percent]);

  const selectedAccount = useMemo(
    () => accounts.find((account) => account.id === selectedAccountId) ?? null,
    [accounts, selectedAccountId]
  );
  const selectedFolder = useMemo(
    () => folders.find((folder) => folder.id === selectedFolderId) ?? null,
    [folders, selectedFolderId]
  );
  const accountSyncState = useMemo(
    () => syncStates.find((state) => state.folder_id === null || state.folder_id === undefined) ?? syncStates[0] ?? null,
    [syncStates]
  );

  const refreshAudits = useCallback(async () => {
    setAudits(await api.getAuditLog(25));
  }, []);

  const refreshAccounts = useCallback(async () => {
    const nextAccounts = await api.listAccounts();
    setAccounts(nextAccounts);
    setSelectedAccountId((current) => current ?? nextAccounts[0]?.id ?? null);
    setStatus(`accounts loaded: ${nextAccounts.length}`);
  }, []);

  const refreshFolders = useCallback(async (accountId: string) => {
    const nextFolders = await api.listFolders(accountId);
    setFolders(nextFolders);
    setSelectedFolderId((current) => {
      if (current && nextFolders.some((folder) => folder.id === current)) return current;
      return nextFolders.find((folder) => folder.role === "inbox")?.id ?? nextFolders[0]?.id ?? null;
    });
  }, []);

  const refreshMessages = useCallback(async (accountId: string, folderId: string | null, searchTerm: string) => {
    const nextMessages = searchTerm.trim()
      ? await api.searchMessages(searchTerm.trim(), 100)
      : await api.listMessages({ account_id: accountId, folder_id: folderId, limit: 100, offset: 0 });
    setMessages(nextMessages);
    setSelectedMessageId((current) => {
      if (current && nextMessages.some((message) => message.id === current)) return current;
      return nextMessages[0]?.id ?? null;
    });
    setStatus(searchTerm.trim() ? `search returned ${nextMessages.length} rows` : `message index loaded: ${nextMessages.length}`);
  }, []);

  const refreshSyncState = useCallback(async (accountId: string) => {
    setSyncStates(await api.getSyncStatus(accountId));
  }, []);

  const refreshAiSettings = useCallback(async () => {
    setAiSettings(await api.getAiSettings());
  }, []);

  const aiHeaderStatus = useMemo(() => {
    if (aiSettings?.enabled && aiSettings.api_key_mask) return "AI READY";
    if (aiSettings) return aiSettings.enabled ? "AI OFFLINE" : "AI DISABLED";
    return "AI OFFLINE";
  }, [aiSettings]);

  useEffect(() => {
    void Promise.all([refreshAccounts().then(refreshAudits), refreshAiSettings()]).catch((error) =>
      setStatus(`startup failed: ${String(error)}`)
    );
  }, [refreshAccounts, refreshAiSettings, refreshAudits]);

  useEffect(() => {
    if (!selectedAccountId) return;
    void Promise.all([refreshFolders(selectedAccountId), refreshSyncState(selectedAccountId)])
      .catch((error) => setStatus(`folder load failed: ${String(error)}`));
  }, [refreshFolders, refreshSyncState, selectedAccountId]);

  useEffect(() => {
    if (!selectedAccountId) return;
    startTransition(() => {
      void refreshMessages(selectedAccountId, selectedFolderId, query).catch((error) => setStatus(`message load failed: ${String(error)}`));
    });
  }, [query, refreshMessages, selectedAccountId, selectedFolderId]);

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  useEffect(() => {
    selectedAccountIdRef.current = selectedAccountId;
  }, [selectedAccountId]);

  useEffect(() => {
    selectedAccountSyncEnabledRef.current = selectedAccount?.sync_enabled ?? true;
  }, [selectedAccount?.sync_enabled]);

  useEffect(() => {
    selectedFolderIdRef.current = selectedFolderId;
  }, [selectedFolderId]);

  useEffect(() => {
    selectedMessageIdRef.current = selectedMessageId;
  }, [selectedMessageId]);

  useEffect(() => {
    queryRef.current = query;
  }, [query]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let isCancelled = false;

    void listen<MailSyncEventPayload>(MAIL_SYNC_EVENT, (event) => {
      void refreshAfterMailSyncEvent({
        payload: event.payload,
        selectedAccountId: selectedAccountIdRef.current,
        selectedFolderId: selectedFolderIdRef.current,
        query: queryRef.current,
        refreshFolders,
        refreshMessages,
        refreshSyncState,
        refreshAudits
      })
        .then((didRefresh) => {
          if (didRefresh) setStatus(`mail sync updated: ${event.payload.reason}`);
        })
        .catch((error) => setStatus(`mail sync refresh failed: ${String(error)}`));
    })
      .then((dispose) => {
        if (isCancelled) {
          dispose();
        } else {
          unlisten = dispose;
        }
      })
      .catch(() => undefined);

    return () => {
      isCancelled = true;
      unlisten?.();
    };
  }, [refreshAudits, refreshFolders, refreshMessages, refreshSyncState]);

  useEffect(() => {
    const runAutoSync = () => {
      if (isAutoSyncRunningRef.current) return;
      isAutoSyncRunningRef.current = true;
      void runAutomaticAccountSync({
        selectedAccountId: selectedAccountIdRef.current,
        selectedFolderId: selectedFolderIdRef.current,
        query: queryRef.current,
        syncEnabled: selectedAccountSyncEnabledRef.current,
        syncAccount: api.syncAccount,
        startAccountWatchers: api.startAccountWatchers,
        refreshFolders,
        refreshMessages,
        refreshSyncState,
        refreshAudits
      })
        .then((result) => {
          if (result.status) setStatus(result.status);
        })
        .catch((error) => setStatus(`auto sync failed: ${String(error)}`))
        .finally(() => {
          isAutoSyncRunningRef.current = false;
        });
    };

    const intervalId = window.setInterval(runAutoSync, AUTO_SYNC_INTERVAL_MS);
    return () => window.clearInterval(intervalId);
  }, [refreshAudits, refreshFolders, refreshMessages, refreshSyncState]);

  useEffect(() => {
    setSelectedMessage(null);
    if (!selectedMessageId) {
      return;
    }
    let isCancelled = false;
    const messageId = selectedMessageId;
    void api
      .getMessage(messageId)
      .then((message) => {
        if (!isCancelled) setSelectedMessage(message);
      })
      .catch(() => {
        if (!isCancelled) setSelectedMessage(messagesRef.current.find((message) => message.id === messageId) ?? null);
      });
    return () => {
      isCancelled = true;
    };
  }, [selectedMessageId]);

  useEffect(() => {
    setAiInsights([]);
    setAiStatus("ai link idle");
    if (!selectedMessageId) return;
    let isCancelled = false;
    const messageId = selectedMessageId;
    void api
      .listAiInsights(messageId)
      .then((insights) => {
        if (!isCancelled) setAiInsights(insights);
      })
      .catch((error) => {
        if (!isCancelled) setAiStatus(`ai insight load failed: ${String(error)}`);
      });
    return () => {
      isCancelled = true;
    };
  }, [selectedMessageId]);

  const handleSync = useCallback(async () => {
    if (!selectedAccountId) return;
    setStatus("syncing account");
    try {
      setStatus(
        await runManualAccountSync({
          accountId: selectedAccountId,
          folderId: selectedFolderId,
          query,
          syncAccount: api.syncAccount,
          startAccountWatchers: api.startAccountWatchers,
          refreshFolders,
          refreshMessages,
          refreshSyncState,
          refreshAudits
        })
      );
    } catch (error) {
      await refreshSyncState(selectedAccountId);
      await refreshAudits();
      setStatus(`sync failed: ${String(error)}`);
    }
  }, [query, refreshAudits, refreshFolders, refreshMessages, refreshSyncState, selectedAccountId, selectedFolderId]);

  const runAction = useCallback(
    async (action: MailActionKind, targetFolderId?: string | null) => {
      if (!selectedAccountId || !selectedMessageId || isActionRunning || isAnalyzing) return;
      setActionRunning(true);
      setStatus(`action running: ${actionLabels[action]}`);
      try {
        await api.executeMailAction({
          action,
          account_id: selectedAccountId,
          message_ids: [selectedMessageId],
          target_folder_id: targetFolderId ?? null
        });
        await refreshFolders(selectedAccountId);
        await refreshMessages(selectedAccountId, selectedFolderId, query);
        await refreshAudits();
        setStatus(`action executed: ${actionLabels[action]}`);
      } catch (error) {
        setStatus(`action failed: ${actionLabels[action]} / ${String(error)}`);
      } finally {
        setActionRunning(false);
      }
    },
    [
      isActionRunning,
      isAnalyzing,
      query,
      refreshAudits,
      refreshFolders,
      refreshMessages,
      selectedAccountId,
      selectedFolderId,
      selectedMessageId
    ]
  );

  const handleAccountConfigSaved = useCallback(
    async (account: MailAccount) => {
      setAccounts((current) => [account, ...current.filter((item) => item.id !== account.id)]);
      setSelectedAccountId(account.id);
      setStatus(
        account.sync_enabled
          ? `account configuration saved, initial sync starting: ${account.email}`
          : `account configuration saved: ${account.email}`
      );
      const nextStatus = await runInitialAccountSync({
        accountId: account.id,
        email: account.email,
        syncEnabled: account.sync_enabled,
        folderId: null,
        query,
        syncAccount: api.syncAccount,
        startAccountWatchers: api.startAccountWatchers,
        refreshFolders,
        refreshMessages,
        refreshSyncState,
        refreshAudits
      });
      setStatus(nextStatus);
    },
    [query, refreshAudits, refreshFolders, refreshMessages, refreshSyncState]
  );

  const handleSent = useCallback(
    async (draft: SendMessageDraft) => {
      const result = await runDirectSendFlow({
        draft,
        selectedFolderId,
        query,
        sendMessage: api.sendMessage,
        refreshFolders,
        refreshMessages,
        refreshAudits
      });
      setStatus(result.status);
      if (!result.ok) {
        throw result.error;
      }
      setComposerOpen(false);
    },
    [query, refreshAudits, refreshFolders, refreshMessages, selectedFolderId]
  );

  const handleAnalyze = useCallback(async () => {
    if (!selectedMessageId || isActionRunning) return;
    const messageId = selectedMessageId;
    setAnalyzing(true);
    setAiStatus("ai analysis running");
    try {
      await api.runAiAnalysis(messageId);
      if (selectedMessageIdRef.current !== messageId) return;
      const insights = await api.listAiInsights(messageId);
      if (selectedMessageIdRef.current !== messageId) return;
      setAiInsights(insights);
      setAiStatus("ai analysis complete");
    } catch (error) {
      if (selectedMessageIdRef.current === messageId) setAiStatus(`ai analysis failed: ${String(error)}`);
    } finally {
      setAnalyzing(false);
    }
  }, [isActionRunning, selectedMessageId]);

  const measureWorkspaceAvailableWidth = useCallback(() => {
    const workspace = mailWorkspaceRef.current;
    if (!workspace) return;
    const rect = workspace.getBoundingClientRect();
    setWorkspaceAvailableWidth(Math.max(0, rect.width - WORKSPACE_DIVIDER_WIDTH));
  }, []);

  useEffect(() => {
    measureWorkspaceAvailableWidth();
    const workspace = mailWorkspaceRef.current;
    if (!workspace) return undefined;

    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => {
            measureWorkspaceAvailableWidth();
          });
    resizeObserver?.observe(workspace);
    window.addEventListener("resize", measureWorkspaceAvailableWidth);
    return () => {
      resizeObserver?.disconnect();
      window.removeEventListener("resize", measureWorkspaceAvailableWidth);
    };
  }, [measureWorkspaceAvailableWidth]);

  const updateWorkspaceSplitFromClientX = useCallback((clientX: number) => {
    const workspace = mailWorkspaceRef.current;
    if (!workspace) return;
    const rect = workspace.getBoundingClientRect();
    const availableWidth = Math.max(0, rect.width - WORKSPACE_DIVIDER_WIDTH);
    if (availableWidth <= 0) return;
    setWorkspaceAvailableWidth(availableWidth);
    const rawPercent = ((clientX - rect.left) / availableWidth) * 100;
    setWorkspaceSplitPercent(
      clampWorkspaceSplitPercent(rawPercent, availableWidth, WORKSPACE_LIST_MIN_WIDTH, WORKSPACE_DETAIL_MIN_WIDTH)
    );
  }, []);

  const stopWorkspaceResize = useCallback(() => {
    setWorkspaceResizing(false);
  }, []);

  const handleWorkspaceDividerPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();
      setWorkspaceResizing(true);
      updateWorkspaceSplitFromClientX(event.clientX);
    },
    [updateWorkspaceSplitFromClientX]
  );

  const handleWorkspaceDividerKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 10 : 2;
    const rect = mailWorkspaceRef.current?.getBoundingClientRect();
    const availableWidth = rect ? Math.max(0, rect.width - WORKSPACE_DIVIDER_WIDTH) : 1000;
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      setWorkspaceSplitPercent((current) =>
        clampWorkspaceSplitPercent(current - step, availableWidth, WORKSPACE_LIST_MIN_WIDTH, WORKSPACE_DETAIL_MIN_WIDTH)
      );
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      setWorkspaceSplitPercent((current) =>
        clampWorkspaceSplitPercent(current + step, availableWidth, WORKSPACE_LIST_MIN_WIDTH, WORKSPACE_DETAIL_MIN_WIDTH)
      );
    } else if (event.key === "Home") {
      event.preventDefault();
      setWorkspaceSplitPercent(clampWorkspaceSplitPercent(35, availableWidth, WORKSPACE_LIST_MIN_WIDTH, WORKSPACE_DETAIL_MIN_WIDTH));
    } else if (event.key === "End") {
      event.preventDefault();
      setWorkspaceSplitPercent(clampWorkspaceSplitPercent(65, availableWidth, WORKSPACE_LIST_MIN_WIDTH, WORKSPACE_DETAIL_MIN_WIDTH));
    }
  }, []);

  useEffect(() => {
    if (!isWorkspaceResizing) return;
    const handlePointerMove = (event: PointerEvent) => {
      event.preventDefault();
      updateWorkspaceSplitFromClientX(event.clientX);
    };
    const handlePointerUp = () => stopWorkspaceResize();
    document.body.classList.add("workspace-resizing");
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp, { once: true });
    window.addEventListener("pointercancel", handlePointerUp, { once: true });
    window.addEventListener("blur", handlePointerUp, { once: true });
    return () => {
      document.body.classList.remove("workspace-resizing");
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerUp);
      window.removeEventListener("blur", handlePointerUp);
    };
  }, [isWorkspaceResizing, stopWorkspaceResize, updateWorkspaceSplitFromClientX]);

  const workspaceStyle = {
    "--workspace-list-percent": `${workspaceSplitModel.percent}%`
  } as CSSProperties;
  const splitValueText = `Message list ${Math.round(workspaceSplitModel.percent)} percent, detail ${Math.round(
    100 - workspaceSplitModel.percent
  )} percent`;

  return (
    <main className={getAppShellClassName(showActivityLog)}>
      <section className="topbar">
        <div className="brand-block" aria-label="AgentMail">
          <TerminalSquare size={21} />
          <div>
            <strong>AGENTMAIL</strong>
            <span>desktop mail command</span>
          </div>
        </div>
        <div className="search-strip">
          <Search size={16} />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search subject, sender, body" />
          {isPending ? <span className="pending-dot">INDEX</span> : null}
        </div>
        <div className="top-actions">
          <button className="icon-button" type="button" onClick={handleSync} disabled={!selectedAccountId} title="Sync account">
            <RefreshCcw size={17} />
          </button>
          <button className="icon-button" type="button" onClick={() => setComposerOpen(true)} disabled={!selectedAccountId} title="Compose">
            <MailPlus size={17} />
          </button>
          <button className="icon-button" type="button" onClick={() => setConfigOpen(true)} title="Configuration">
            <Settings size={17} />
          </button>
        </div>
      </section>

      <section className="workspace">
        <aside className="account-rail">
          <div className="rail-title">ACCOUNTS</div>
          {accounts.map((accountItem) => (
            <button
              className={`account-tile ${accountItem.id === selectedAccountId ? "active" : ""}`}
              key={accountItem.id}
              type="button"
              onClick={() => setSelectedAccountId(accountItem.id)}
            >
              <Mail size={16} />
              <span>{accountItem.display_name || accountItem.email}</span>
              <small>{accountItem.email}</small>
            </button>
          ))}
          <div className="rail-title folders-label">FOLDERS</div>
          <nav className="folder-list" aria-label="Mail folders">
            {folders.map((folder) => {
              const Icon = roleIcon[folder.role];
              return (
                <button
                  className={`folder-row ${folder.id === selectedFolderId ? "active" : ""}`}
                  key={folder.id}
                  type="button"
                  onClick={() => setSelectedFolderId(folder.id)}
                >
                  <Icon size={15} />
                  <span>{folder.name}</span>
                  <code>{formatFolderCount(folder)}</code>
                </button>
              );
            })}
          </nav>
        </aside>

        <div className="mail-workspace" ref={mailWorkspaceRef} style={workspaceStyle}>
          <section className="message-column" aria-label="Messages">
            <div className="column-header">
              <div>
                <span>{selectedFolder?.name ?? "NO FOLDER"}</span>
                <strong>{messages.length} ROWS</strong>
              </div>
              <div className="health-chip">
                <ShieldCheck size={14} />
                {aiHeaderStatus}
              </div>
            </div>
            <div className="message-list">
              {messages.map((message) => (
                <button
                  className={`message-row ${message.id === selectedMessageId ? "active" : ""} ${message.flags.is_read ? "read" : "unread"}`}
                  key={message.id}
                  type="button"
                  onClick={() => setSelectedMessageId(message.id)}
                >
                  <span className="row-status">{message.flags.is_starred ? <Star size={14} fill="currentColor" /> : <Clock3 size={14} />}</span>
                  <span className="row-main">
                    <strong>{message.subject}</strong>
                    <small>{message.sender}</small>
                    <em>{message.body_preview}</em>
                  </span>
                  <time>{formatTime(message.received_at)}</time>
                </button>
              ))}
              {messages.length === 0 ? <div className="empty-state">No indexed mail in this folder.</div> : null}
            </div>
          </section>

          <div
            className={`workspace-divider ${isWorkspaceResizing ? "active" : ""}`}
            role="separator"
            aria-label="Resize message list and detail panes"
            aria-orientation="vertical"
            aria-valuemin={Math.round(workspaceSplitModel.minPercent)}
            aria-valuemax={Math.round(workspaceSplitModel.maxPercent)}
            aria-valuenow={Math.round(workspaceSplitModel.percent)}
            aria-valuetext={splitValueText}
            tabIndex={0}
            onPointerDown={handleWorkspaceDividerPointerDown}
            onLostPointerCapture={stopWorkspaceResize}
            onKeyDown={handleWorkspaceDividerKeyDown}
          />

          <section className="detail-pane" aria-label="Message detail">
            {selectedMessage ? (
              <>
                <div className="detail-toolbar">
                  <button
                    type="button"
                    onClick={() => runAction(selectedMessage.flags.is_read ? "mark_unread" : "mark_read")}
                    disabled={isActionRunning || isAnalyzing}
                  >
                    <CheckCheck size={15} />
                    {selectedMessage.flags.is_read ? "UNREAD" : "READ"}
                  </button>
                  <button
                    type="button"
                    onClick={() => runAction(selectedMessage.flags.is_starred ? "unstar" : "star")}
                    disabled={isActionRunning || isAnalyzing}
                  >
                    <Star size={15} />
                    {selectedMessage.flags.is_starred ? "UNSTAR" : "STAR"}
                  </button>
                  <button type="button" onClick={() => runAction("delete")} disabled={isActionRunning || isAnalyzing}>
                    <Trash2 size={15} />
                    DELETE
                  </button>
                </div>
                <div className="detail-scroll">
                  <article className="message-detail">
                    <div className="message-heading">
                      <span className={selectedMessage.flags.is_read ? "read-dot" : "unread-dot"} />
                      <h1>{selectedMessage.subject}</h1>
                    </div>
                    <dl className="metadata-grid">
                      <div>
                        <dt>FROM</dt>
                        <dd>{selectedMessage.sender}</dd>
                      </div>
                      <div>
                        <dt>TO</dt>
                        <dd>{selectedMessage.recipients.join(", ")}</dd>
                      </div>
                      <div>
                        <dt>UID</dt>
                        <dd>{selectedMessage.uid ?? "LOCAL"}</dd>
                      </div>
                      <div>
                        <dt>SIZE</dt>
                        <dd>{formatSize(selectedMessage.size_bytes)}</dd>
                      </div>
                    </dl>
                    <pre className="body-block">{selectedMessage.body ?? selectedMessage.body_preview}</pre>
                    {selectedMessage.attachments.length > 0 ? (
                      <div className="attachment-strip">
                        {selectedMessage.attachments.map((attachment) => (
                          <span key={attachment.id}>
                            <Database size={14} />
                            {attachment.filename}
                          </span>
                        ))}
                      </div>
                    ) : null}
                  </article>
                  <AiPanel
                    settings={aiSettings}
                    insights={aiInsights}
                    status={aiStatus}
                    isAnalyzing={isAnalyzing}
                    isActionRunning={isActionRunning}
                    onAnalyze={handleAnalyze}
                  />
                </div>
              </>
            ) : (
              <div className="empty-detail">
                <BellDot size={24} />
                Select a message to inspect headers and body.
              </div>
            )}
          </section>
        </div>
      </section>

      {showActivityLog ? (
        <footer className="status-console">
          <section className="console-panel audit-feed">
            <header>AUDIT / ACTIVITY LOG</header>
            <p>{accountSyncState?.error_message ?? status}</p>
            {audits.slice(0, 8).map((audit) => (
              <code key={audit.id}>{formatAuditLine(audit)}</code>
            ))}
          </section>
        </footer>
      ) : null}

      {isComposerOpen && selectedAccount ? <Composer account={selectedAccount} onClose={() => setComposerOpen(false)} onSent={handleSent} /> : null}
      {isConfigOpen ? (
        <ConfigurationModal
          accounts={accounts}
          selectedAccountId={selectedAccountId}
          settings={aiSettings}
          themeMode={themeMode}
          showActivityLog={showActivityLog}
          onThemeModeChange={setThemeMode}
          onShowActivityLogChange={setShowActivityLog}
          onClose={() => setConfigOpen(false)}
          onAccountSaved={handleAccountConfigSaved}
          onAiSettingsSaved={refreshAiSettings}
        />
      ) : null}
    </main>
  );
}

interface AiPanelProps {
  settings: AiSettingsView | null;
  insights: AiInsight[];
  status: string;
  isAnalyzing: boolean;
  isActionRunning: boolean;
  onAnalyze: () => Promise<void>;
}

function AiPanel({ settings, insights, status, isAnalyzing, isActionRunning, onAnalyze }: AiPanelProps) {
  const latest = insights[0] ?? null;
  const keyStatus = settings?.api_key_mask ? settings.api_key_mask : "not set";

  return (
    <aside className="ai-panel">
      <header>
        <div>
          <PanelRight size={15} />
          <span>AI</span>
        </div>
      </header>
      <div className="ai-status-grid">
        <span>PROVIDER</span>
        <code>{settings?.provider_name ?? "not set"}</code>
        <span>MODEL</span>
        <code>{settings?.model || "not set"}</code>
        <span>KEY</span>
        <code>{keyStatus}</code>
      </div>
      <div className="ai-actions">
        <button type="button" onClick={onAnalyze} disabled={isAnalyzing || isActionRunning || !settings?.enabled}>
          {isAnalyzing ? "RUNNING" : "AI ANALYZE"}
        </button>
        <span>{settings?.enabled ? "enabled" : "disabled"}</span>
      </div>
      {latest ? (
        <section className="ai-result">
          <div>
            <strong>{latest.category || "uncategorized"}</strong>
            <code>{latest.priority}</code>
          </div>
          <p>{latest.summary}</p>
          {latest.todos.length > 0 ? (
            <ul className="ai-todo-list">
              {latest.todos.map((todo) => (
                <li key={todo}>{todo}</li>
              ))}
            </ul>
          ) : null}
          {latest.reply_draft ? <pre>{latest.reply_draft}</pre> : null}
        </section>
      ) : (
        <section className="ai-result empty">No insight for this message.</section>
      )}
      {insights.length > 0 ? (
        <div className="ai-history">
          {insights.map((insight) => (
            <div key={insight.id}>
              <time>{formatTime(insight.created_at)}</time>
              <span>{insight.category || "uncategorized"}</span>
              <code>{insight.priority}</code>
            </div>
          ))}
        </div>
      ) : null}
      <p className="ai-status-text">{status}</p>
    </aside>
  );
}

interface ConfigurationModalProps {
  accounts: MailAccount[];
  selectedAccountId: string | null;
  settings: AiSettingsView | null;
  themeMode: ThemeMode;
  showActivityLog: boolean;
  onThemeModeChange: (mode: ThemeMode) => void;
  onShowActivityLogChange: (visible: boolean) => void;
  onClose: () => void;
  onAccountSaved: (account: MailAccount) => Promise<void>;
  onAiSettingsSaved: () => Promise<void>;
}

type ConfigTab = "accounts" | "ai" | "display";

function accountConfigToForm(config: AccountConfigView): SaveAccountConfigRequest {
  return {
    id: config.id,
    display_name: config.display_name,
    email: config.email,
    password: config.password,
    imap_host: config.imap_host,
    imap_port: config.imap_port,
    imap_tls: config.imap_tls,
    smtp_host: config.smtp_host,
    smtp_port: config.smtp_port,
    smtp_tls: config.smtp_tls,
    sync_enabled: config.sync_enabled
  };
}

function ConfigurationModal({
  accounts,
  selectedAccountId,
  settings,
  themeMode,
  showActivityLog,
  onThemeModeChange,
  onShowActivityLogChange,
  onClose,
  onAccountSaved,
  onAiSettingsSaved
}: ConfigurationModalProps) {
  const [activeTab, setActiveTab] = useState<ConfigTab>("accounts");
  const [selectedId, setSelectedId] = useState<string | null>(selectedAccountId ?? accounts[0]?.id ?? null);
  const [accountForm, setAccountForm] = useState<SaveAccountConfigRequest>({ ...defaultAccountConfigForm });
  const [accountStatus, setAccountStatus] = useState("select account or add new");
  const [isAccountBusy, setAccountBusy] = useState(false);
  const [aiForm, setAiForm] = useState<SaveAiSettingsRequest>({
    provider_name: settings?.provider_name ?? "openai-compatible",
    base_url: settings?.base_url ?? "https://api.openai.com/v1",
    model: settings?.model ?? "",
    api_key: "",
    enabled: settings?.enabled ?? false
  });
  const [aiStatus, setAiStatus] = useState(settings?.api_key_mask ? `key saved: ${settings.api_key_mask}` : "key not set");
  const [isAiBusy, setAiBusy] = useState(false);

  useEffect(() => {
    if (selectedId && !accounts.some((account) => account.id === selectedId)) {
      setSelectedId(accounts[0]?.id ?? null);
    }
  }, [accounts, selectedId]);

  useEffect(() => {
    if (!selectedId) {
      setAccountForm({ ...defaultAccountConfigForm });
      setAccountStatus("new account");
      return;
    }

    let isCancelled = false;
    setAccountStatus("loading account config");
    void api
      .getAccountConfig(selectedId)
      .then((config) => {
        if (isCancelled) return;
        setAccountForm(accountConfigToForm(config));
        setAccountStatus(`loaded ${config.email}`);
      })
      .catch((error) => {
        if (!isCancelled) setAccountStatus(`load failed: ${String(error)}`);
      });

    return () => {
      isCancelled = true;
    };
  }, [selectedId]);

  const updateAccount = (field: keyof SaveAccountConfigRequest, value: string | number | boolean | null) => {
    setAccountForm((current) => ({ ...current, [field]: value }));
  };

  const updateAi = (field: keyof SaveAiSettingsRequest, value: string | boolean) => {
    setAiForm((current) => ({ ...current, [field]: value }));
  };

  const startNewAccount = () => {
    setSelectedId(null);
    setAccountForm({ ...defaultAccountConfigForm });
    setAccountStatus("new account");
  };

  const testAccount = async () => {
    setAccountBusy(true);
    setAccountStatus("testing account connection");
    try {
      const result = await api.testAccountConnection(accountForm);
      setAccountStatus(`${result.imap_ok ? "IMAP OK" : "IMAP FAIL"} / ${result.smtp_ok ? "SMTP OK" : "SMTP FAIL"} / ${result.message}`);
    } catch (error) {
      setAccountStatus(`test failed: ${String(error)}`);
    } finally {
      setAccountBusy(false);
    }
  };

  const submitAccount = async (event: FormEvent) => {
    event.preventDefault();
    setAccountBusy(true);
    setAccountStatus("saving account config");
    try {
      const account = await api.saveAccountConfig(accountForm);
      setSelectedId(account.id);
      await onAccountSaved(account);
      setAccountStatus(`saved ${account.email}`);
    } catch (error) {
      setAccountStatus(`save failed: ${String(error)}`);
    } finally {
      setAccountBusy(false);
    }
  };

  const submitAi = async (event: FormEvent) => {
    event.preventDefault();
    setAiBusy(true);
    setAiStatus("saving ai settings");
    try {
      await api.saveAiSettings({
        ...aiForm,
        api_key: aiForm.api_key?.trim() ? aiForm.api_key : null
      });
      await onAiSettingsSaved();
      setAiStatus("ai settings saved");
    } catch (error) {
      setAiStatus(`save failed: ${String(error)}`);
    } finally {
      setAiBusy(false);
    }
  };

  const clearAi = async () => {
    setAiBusy(true);
    setAiStatus("clearing ai settings");
    try {
      await api.clearAiSettings();
      await onAiSettingsSaved();
      setAiForm({
        provider_name: "openai-compatible",
        base_url: "https://api.openai.com/v1",
        model: "",
        api_key: "",
        enabled: false
      });
      setAiStatus("key not set");
    } catch (error) {
      setAiStatus(`clear failed: ${String(error)}`);
    } finally {
      setAiBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <section className="modal-panel configuration-panel" role="dialog" aria-modal="true" aria-label="Configuration">
        <header>
          <h2>CONFIGURATION</h2>
          <button className="icon-button" type="button" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </header>
        <div className="config-tabs" role="tablist" aria-label="Configuration sections">
          <button className={activeTab === "accounts" ? "active" : ""} type="button" onClick={() => setActiveTab("accounts")}>
            MAIL ACCOUNTS
          </button>
          <button className={activeTab === "ai" ? "active" : ""} type="button" onClick={() => setActiveTab("ai")}>
            AI MODEL
          </button>
          <button className={activeTab === "display" ? "active" : ""} type="button" onClick={() => setActiveTab("display")}>
            DISPLAY
          </button>
        </div>

        {activeTab === "accounts" ? (
          <form className="config-body" onSubmit={submitAccount}>
            <aside className="config-account-list">
              <button type="button" onClick={startNewAccount}>
                ADD ACCOUNT
              </button>
              {accounts.map((account) => (
                <button
                  className={account.id === selectedId ? "active" : ""}
                  key={account.id}
                  type="button"
                  onClick={() => setSelectedId(account.id)}
                >
                  <span>{account.display_name || account.email}</span>
                  <small>{account.email}</small>
                </button>
              ))}
            </aside>
            <section className="config-form-grid">
              <label>
                Display
                <input value={accountForm.display_name} onChange={(event) => updateAccount("display_name", event.target.value)} required />
              </label>
              <label>
                Email
                <input type="email" value={accountForm.email} onChange={(event) => updateAccount("email", event.target.value)} required />
              </label>
              <label>
                Password
                <input type="password" value={accountForm.password} onChange={(event) => updateAccount("password", event.target.value)} required />
              </label>
              <label>
                IMAP Host
                <input value={accountForm.imap_host} onChange={(event) => updateAccount("imap_host", event.target.value)} required />
              </label>
              <label>
                IMAP Port
                <input type="number" value={accountForm.imap_port} onChange={(event) => updateAccount("imap_port", Number(event.target.value))} required />
              </label>
              <label>
                SMTP Host
                <input value={accountForm.smtp_host} onChange={(event) => updateAccount("smtp_host", event.target.value)} required />
              </label>
              <label>
                SMTP Port
                <input type="number" value={accountForm.smtp_port} onChange={(event) => updateAccount("smtp_port", Number(event.target.value))} required />
              </label>
              <label className="checkbox-row">
                <input type="checkbox" checked={accountForm.imap_tls} onChange={(event) => updateAccount("imap_tls", event.target.checked)} />
                IMAP TLS
              </label>
              <label className="checkbox-row">
                <input type="checkbox" checked={accountForm.smtp_tls} onChange={(event) => updateAccount("smtp_tls", event.target.checked)} />
                SMTP TLS
              </label>
              <label className="checkbox-row">
                <input type="checkbox" checked={accountForm.sync_enabled} onChange={(event) => updateAccount("sync_enabled", event.target.checked)} />
                Sync
              </label>
              <div className="modal-status">{accountStatus}</div>
              <footer>
                <button type="button" onClick={testAccount} disabled={isAccountBusy}>
                  TEST
                </button>
                <button type="submit" disabled={isAccountBusy}>
                  {isAccountBusy ? "SAVING" : "SAVE"}
                </button>
              </footer>
            </section>
          </form>
        ) : activeTab === "ai" ? (
          <form className="config-ai-form" onSubmit={submitAi}>
            <label>
              Provider
              <input value={aiForm.provider_name} onChange={(event) => updateAi("provider_name", event.target.value)} required />
            </label>
            <label>
              Base URL
              <input value={aiForm.base_url} onChange={(event) => updateAi("base_url", event.target.value)} required />
            </label>
            <label>
              Model
              <input value={aiForm.model} onChange={(event) => updateAi("model", event.target.value)} required />
            </label>
            <label>
              API Key
              <input
                type="password"
                value={aiForm.api_key ?? ""}
                onChange={(event) => updateAi("api_key", event.target.value)}
                placeholder={settings?.api_key_mask ?? "sk-..."}
              />
            </label>
            <label className="checkbox-row">
              <input type="checkbox" checked={aiForm.enabled} onChange={(event) => updateAi("enabled", event.target.checked)} />
              Enabled
            </label>
            <div className="modal-status">{aiStatus}</div>
            <footer>
              <button type="button" onClick={clearAi} disabled={isAiBusy}>
                CLEAR
              </button>
              <button type="submit" disabled={isAiBusy}>
                {isAiBusy ? "SAVING" : "SAVE"}
              </button>
            </footer>
          </form>
        ) : (
          <section className="config-display-panel">
            <div className="theme-switch-row">
              <div>
                <span>DISPLAY MODE</span>
                <strong>{themeMode === "dark" ? "DARK INDUSTRIAL" : "ARCHIVE BEIGE"}</strong>
              </div>
              <button
                type="button"
                onClick={() => onThemeModeChange(getNextThemeMode(themeMode))}
                aria-pressed={themeMode === "light"}
                title={themeMode === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              >
                {themeMode === "dark" ? <Sun size={16} /> : <Moon size={16} />}
                {themeMode === "dark" ? "LIGHT" : "DARK"}
              </button>
            </div>
            <div className="theme-switch-row">
              <div>
                <span>ACTIVITY LOG</span>
                <strong>{showActivityLog ? "VISIBLE" : "HIDDEN"}</strong>
              </div>
              <button
                type="button"
                onClick={() => onShowActivityLogChange(!showActivityLog)}
                aria-pressed={showActivityLog}
                title={showActivityLog ? "Hide activity log" : "Show activity log"}
              >
                <PanelRight size={16} />
                {showActivityLog ? "HIDE" : "SHOW"}
              </button>
            </div>
            <div className="theme-swatch-grid" aria-label="Theme color preview">
              <span className="theme-swatch swatch-surface" />
              <span className="theme-swatch swatch-panel" />
              <span className="theme-swatch swatch-accent" />
              <span className="theme-swatch swatch-danger" />
            </div>
          </section>
        )}
      </section>
    </div>
  );
}

interface ComposerProps {
  account: MailAccount;
  onClose: () => void;
  onSent: (draft: SendMessageDraft) => Promise<void>;
}

function Composer({ account, onClose, onSent }: ComposerProps) {
  const [draft, setDraft] = useState<SendMessageDraft>({
    account_id: account.id,
    to: [],
    cc: [],
    subject: "",
    body: ""
  });
  const [toField, setToField] = useState("");
  const [ccField, setCcField] = useState("");
  const [isSending, setSending] = useState(false);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setSending(true);
    try {
      await onSent({
        ...draft,
        to: splitAddresses(toField),
        cc: splitAddresses(ccField)
      });
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <form className="modal-panel composer-panel" onSubmit={submit}>
        <header>
          <h2>COMPOSE</h2>
          <button className="icon-button" type="button" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </header>
        <label>
          From
          <input value={account.email} readOnly />
        </label>
        <label>
          To
          <input value={toField} onChange={(event) => setToField(event.target.value)} placeholder="name@example.com, ops@example.com" required />
        </label>
        <label>
          Cc
          <input value={ccField} onChange={(event) => setCcField(event.target.value)} />
        </label>
        <label>
          Subject
          <input value={draft.subject} onChange={(event) => setDraft((current) => ({ ...current, subject: event.target.value }))} required />
        </label>
        <label>
          Body
          <textarea value={draft.body} onChange={(event) => setDraft((current) => ({ ...current, body: event.target.value }))} required />
        </label>
        <footer>
          <button type="button" onClick={onClose}>
            CANCEL
          </button>
          <button type="submit" disabled={isSending}>
            {isSending ? "SENDING" : "SEND"}
          </button>
        </footer>
      </form>
    </div>
  );
}

function splitAddresses(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function formatTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "unknown";
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(date);
}

function formatSize(value?: number | null) {
  if (!value) return "0 B";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}
