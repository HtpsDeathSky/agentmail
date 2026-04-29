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
  MailActionKind,
  MailFolder,
  MailMessage,
  SaveAccountConfigRequest,
  SaveAiSettingsRequest,
  SendMessageDraft,
} from "./api";
import {
  applyThemeModeToDocument,
  DEFAULT_WORKSPACE_SPLIT_PERCENT,
  getNextThemeMode,
  readStoredActivityLogVisibility,
  readStoredThemeMode,
  readStoredWorkspaceSplitPercent,
  writeStoredActivityLogVisibility,
  writeStoredWorkspaceSplitPercent,
  type ThemeMode
} from "./lib/storage";
import { actionLabels } from "./lib/mailActions";
import { formatFolderCount, formatSize, formatTime } from "./lib/format";
import {
  refreshAfterMailSyncEvent,
  runDirectSendFlow,
  runInitialAccountSync,
  runManualAccountSync,
  type MailSyncEventPayload
} from "./lib/syncFlows";
import { getManualSyncButtonState } from "./lib/syncUi";
import { appendActivityLogEntry, buildActivityLogText, type ActivityLogEntry } from "./lib/activityLog";

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
const WORKSPACE_LIST_MIN_WIDTH = 320;
const WORKSPACE_DETAIL_MIN_WIDTH = 420;
const WORKSPACE_DIVIDER_WIDTH = 8;

const roleIcon = {
  inbox: Inbox,
  sent: Send,
  archive: Archive,
  trash: Trash2,
  drafts: MailPlus,
  junk: CircleAlert,
  custom: Folder
};

export function getAppShellClassName(showActivityLog: boolean) {
  return showActivityLog ? "app-shell activity-log-visible" : "app-shell";
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

export function App() {
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [folders, setFolders] = useState<MailFolder[]>([]);
  const [messages, setMessages] = useState<MailMessage[]>([]);
  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(null);
  const [selectedFolderId, setSelectedFolderId] = useState<string | null>(null);
  const [selectedMessageId, setSelectedMessageId] = useState<string | null>(null);
  const [selectedMessage, setSelectedMessage] = useState<MailMessage | null>(null);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState("backend link idle");
  const [isConfigOpen, setConfigOpen] = useState(false);
  const [isComposerOpen, setComposerOpen] = useState(false);
  const [aiSettings, setAiSettings] = useState<AiSettingsView | null>(null);
  const [aiInsights, setAiInsights] = useState<AiInsight[]>([]);
  const [isAnalyzing, setAnalyzing] = useState(false);
  const [isActionRunning, setActionRunning] = useState(false);
  const [isManualSyncing, setManualSyncing] = useState(false);
  const [activityLogEntries, setActivityLogEntries] = useState<ActivityLogEntry[]>([]);
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
  const selectedFolderIdRef = useRef<string | null>(null);
  const selectedMessageIdRef = useRef<string | null>(null);
  const queryRef = useRef("");

  const appendActivityLog = useCallback((message: string) => {
    setActivityLogEntries((current) => appendActivityLogEntry(current, message));
  }, []);

  useEffect(() => {
    if (typeof document === "undefined") return;
    applyThemeModeToDocument(document.documentElement, typeof window === "undefined" ? null : window.localStorage, themeMode);
  }, [themeMode]);

  useEffect(() => {
    appendActivityLog("app startup");
  }, [appendActivityLog]);

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
  const refreshAudits = useCallback(async () => {
    await api.getAuditLog(25);
  }, []);

  const refreshAccounts = useCallback(async () => {
    const nextAccounts = await api.listAccounts();
    setAccounts(nextAccounts);
    setSelectedAccountId((current) => current ?? nextAccounts[0]?.id ?? null);
    setStatus(`accounts loaded: ${nextAccounts.length}`);
    appendActivityLog(`accounts loaded: ${nextAccounts.length}`);
  }, [appendActivityLog]);

  const refreshFolders = useCallback(async (accountId: string) => {
    const nextFolders = await api.listFolders(accountId);
    setFolders(nextFolders);
    setSelectedFolderId((current) => {
      if (current && nextFolders.some((folder) => folder.id === current)) return current;
      return nextFolders.find((folder) => folder.role === "inbox")?.id ?? nextFolders[0]?.id ?? null;
    });
    appendActivityLog(`folders loaded: ${accountId} / ${nextFolders.length}`);
  }, [appendActivityLog]);

  const refreshMessages = useCallback(async (accountId: string, folderId: string | null, searchTerm: string) => {
    const nextMessages = searchTerm.trim()
      ? await api.searchMessages(searchTerm.trim(), 100)
      : await api.listMessages({ account_id: accountId, folder_id: folderId, limit: 100, offset: 0 });
    setMessages(nextMessages);
    setSelectedMessageId((current) => {
      if (current && nextMessages.some((message) => message.id === current)) return current;
      return nextMessages[0]?.id ?? null;
    });
    const nextStatus = searchTerm.trim() ? `search returned ${nextMessages.length} rows` : `message index loaded: ${nextMessages.length}`;
    setStatus(nextStatus);
    appendActivityLog(nextStatus);
  }, [appendActivityLog]);

  const refreshSyncState = useCallback(async (accountId: string) => {
    await api.getSyncStatus(accountId);
    appendActivityLog(`sync state loaded: ${accountId}`);
  }, [appendActivityLog]);

  const refreshAiSettings = useCallback(async () => {
    setAiSettings(await api.getAiSettings());
    appendActivityLog("ai settings loaded");
  }, [appendActivityLog]);

  const aiHeaderStatus = useMemo(() => {
    if (aiSettings?.enabled && aiSettings.api_key_mask) return "AI READY";
    if (aiSettings) return aiSettings.enabled ? "AI OFFLINE" : "AI DISABLED";
    return "AI OFFLINE";
  }, [aiSettings]);

  useEffect(() => {
    appendActivityLog("startup refresh started");
    void Promise.all([refreshAccounts().then(refreshAudits), refreshAiSettings()])
      .then(() => appendActivityLog("startup refresh complete"))
      .catch((error) => {
        const nextStatus = `startup failed: ${String(error)}`;
        setStatus(nextStatus);
        appendActivityLog(nextStatus);
      });
  }, [appendActivityLog, refreshAccounts, refreshAiSettings, refreshAudits]);

  useEffect(() => {
    if (!selectedAccountId) return;
    void Promise.all([refreshFolders(selectedAccountId), refreshSyncState(selectedAccountId)])
      .catch((error) => {
        const nextStatus = `folder load failed: ${String(error)}`;
        setStatus(nextStatus);
        appendActivityLog(nextStatus);
      });
  }, [appendActivityLog, refreshFolders, refreshSyncState, selectedAccountId]);

  useEffect(() => {
    if (!selectedAccountId) return;
    startTransition(() => {
      void refreshMessages(selectedAccountId, selectedFolderId, query).catch((error) => {
        const nextStatus = `message load failed: ${String(error)}`;
        setStatus(nextStatus);
        appendActivityLog(nextStatus);
      });
    });
  }, [appendActivityLog, query, refreshMessages, selectedAccountId, selectedFolderId]);

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  useEffect(() => {
    selectedAccountIdRef.current = selectedAccountId;
  }, [selectedAccountId]);

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
      appendActivityLog(
        `mail sync event: ${event.payload.reason} / account ${event.payload.account_id} / folder ${
          event.payload.folder_id ?? "account"
        }${event.payload.message ? ` / ${event.payload.message}` : ""}`
      );
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
          if (didRefresh) {
            const nextStatus = `mail sync updated: ${event.payload.reason}`;
            setStatus(nextStatus);
            appendActivityLog(nextStatus);
          } else {
            appendActivityLog(`mail sync event ignored for inactive account: ${event.payload.account_id}`);
          }
        })
        .catch((error) => {
          const nextStatus = `mail sync refresh failed: ${String(error)}`;
          setStatus(nextStatus);
          appendActivityLog(nextStatus);
        });
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
  }, [appendActivityLog, refreshAudits, refreshFolders, refreshMessages, refreshSyncState]);

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
    if (!selectedAccountId || isManualSyncing) return;
    setManualSyncing(true);
    setStatus("sync running");
    appendActivityLog(`manual sync started: ${selectedAccountId}`);
    try {
      const nextStatus = await runManualAccountSync({
        accountId: selectedAccountId,
        folderId: selectedFolderId,
        query,
        syncAccount: (accountId) => api.syncAccount(accountId, "manual_sync"),
        refreshFolders,
        refreshMessages,
        refreshSyncState,
        refreshAudits
      });
      setStatus(nextStatus);
      appendActivityLog(nextStatus);
    } catch (error) {
      await refreshSyncState(selectedAccountId);
      await refreshAudits();
      const nextStatus = `sync failed: ${String(error)}`;
      setStatus(nextStatus);
      appendActivityLog(nextStatus);
    } finally {
      setManualSyncing(false);
    }
  }, [
    appendActivityLog,
    isManualSyncing,
    query,
    refreshAudits,
    refreshFolders,
    refreshMessages,
    refreshSyncState,
    selectedAccountId,
    selectedFolderId
  ]);

  const runAction = useCallback(
    async (action: MailActionKind, targetFolderId?: string | null) => {
      if (!selectedAccountId || !selectedMessageId || isActionRunning || isAnalyzing) return;
      setActionRunning(true);
      setStatus(`action running: ${actionLabels[action]}`);
      appendActivityLog(`action started: ${actionLabels[action]} / ${selectedMessageId}`);
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
        const nextStatus = `action executed: ${actionLabels[action]}`;
        setStatus(nextStatus);
        appendActivityLog(nextStatus);
      } catch (error) {
        const nextStatus = `action failed: ${actionLabels[action]} / ${String(error)}`;
        setStatus(nextStatus);
        appendActivityLog(nextStatus);
      } finally {
        setActionRunning(false);
      }
    },
    [
      appendActivityLog,
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
      const saveStatus = account.sync_enabled
        ? `account configuration saved, initial sync starting: ${account.email}`
        : `account configuration saved: ${account.email}`;
      setStatus(saveStatus);
      appendActivityLog(saveStatus);
      const nextStatus = await runInitialAccountSync({
        accountId: account.id,
        email: account.email,
        syncEnabled: account.sync_enabled,
        folderId: null,
        query,
        syncAccount: (accountId) => api.syncAccount(accountId, "account_saved_sync"),
        refreshFolders,
        refreshMessages,
        refreshSyncState,
        refreshAudits
      });
      setStatus(nextStatus);
      appendActivityLog(nextStatus);
    },
    [appendActivityLog, query, refreshAudits, refreshFolders, refreshMessages, refreshSyncState]
  );

  const handleSent = useCallback(
    async (draft: SendMessageDraft) => {
      appendActivityLog(`send started: ${draft.to.join(", ") || "no recipients"}`);
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
      appendActivityLog(result.status);
      if (!result.ok) {
        throw result.error;
      }
      setComposerOpen(false);
    },
    [appendActivityLog, query, refreshAudits, refreshFolders, refreshMessages, selectedFolderId]
  );

  const handleAnalyze = useCallback(async () => {
    if (!selectedMessageId || isActionRunning) return;
    const messageId = selectedMessageId;
    setAnalyzing(true);
    setAiStatus("ai analysis running");
    appendActivityLog(`ai analysis started: ${messageId}`);
    try {
      await api.runAiAnalysis(messageId);
      if (selectedMessageIdRef.current !== messageId) return;
      const insights = await api.listAiInsights(messageId);
      if (selectedMessageIdRef.current !== messageId) return;
      setAiInsights(insights);
      setAiStatus("ai analysis complete");
      appendActivityLog(`ai analysis complete: ${messageId}`);
    } catch (error) {
      if (selectedMessageIdRef.current === messageId) {
        const nextStatus = `ai analysis failed: ${String(error)}`;
        setAiStatus(nextStatus);
        appendActivityLog(nextStatus);
      }
    } finally {
      setAnalyzing(false);
    }
  }, [appendActivityLog, isActionRunning, selectedMessageId]);

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
  const manualSyncButton = getManualSyncButtonState(selectedAccountId, isManualSyncing);
  const activityLogText = buildActivityLogText(activityLogEntries);

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
          <button
            className={manualSyncButton.className}
            type="button"
            onClick={handleSync}
            disabled={manualSyncButton.disabled}
            title={manualSyncButton.title}
          >
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
            <textarea
              aria-label="Activity log history"
              className="activity-log-textbox"
              placeholder="No session activity yet."
              readOnly
              value={activityLogText}
            />
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
