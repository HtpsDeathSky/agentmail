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
  PanelRight,
  RefreshCcw,
  Search,
  Send,
  Settings,
  ShieldCheck,
  Star,
  TerminalSquare,
  Trash2,
  X
} from "lucide-react";
import { FormEvent, useCallback, useEffect, useMemo, useRef, useState, useTransition } from "react";
import {
  AddAccountRequest,
  AiInsight,
  AiSettingsView,
  api,
  MailAccount,
  MailActionAudit,
  MailActionKind,
  MailFolder,
  MailMessage,
  PendingMailAction,
  SaveAiSettingsRequest,
  SendMessageDraft,
  SyncState
} from "./api";

const defaultAccountForm: AddAccountRequest = {
  display_name: "",
  email: "",
  password: "",
  imap_host: "",
  imap_port: 993,
  imap_tls: true,
  smtp_host: "",
  smtp_port: 465,
  smtp_tls: true
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

export function formatSendQueuedStatus(recipients: string[]) {
  return `send queued for ${recipients.join(", ")} / confirm SEND in PENDING ACTIONS`;
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
  const [pendingActions, setPendingActions] = useState<PendingMailAction[]>([]);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState("backend link idle");
  const [isAccountModalOpen, setAccountModalOpen] = useState(false);
  const [isComposerOpen, setComposerOpen] = useState(false);
  const [aiSettings, setAiSettings] = useState<AiSettingsView | null>(null);
  const [aiInsights, setAiInsights] = useState<AiInsight[]>([]);
  const [isAnalyzing, setAnalyzing] = useState(false);
  const [isActionRunning, setActionRunning] = useState(false);
  const [aiStatus, setAiStatus] = useState("ai link idle");
  const [isAiSettingsOpen, setAiSettingsOpen] = useState(false);
  const [isPending, startTransition] = useTransition();
  const messagesRef = useRef<MailMessage[]>([]);
  const selectedMessageIdRef = useRef<string | null>(null);

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

  const refreshPendingActions = useCallback(async (accountId: string | null) => {
    setPendingActions(await api.listPendingActions(accountId));
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
    void Promise.all([refreshFolders(selectedAccountId), refreshSyncState(selectedAccountId), refreshPendingActions(selectedAccountId)])
      .catch((error) => setStatus(`folder load failed: ${String(error)}`));
  }, [refreshFolders, refreshPendingActions, refreshSyncState, selectedAccountId]);

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
    selectedMessageIdRef.current = selectedMessageId;
  }, [selectedMessageId]);

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
      const summary = await api.syncAccount(selectedAccountId);
      await refreshFolders(selectedAccountId);
      await refreshMessages(selectedAccountId, selectedFolderId, query);
      await refreshSyncState(selectedAccountId);
      await refreshAudits();
      await refreshPendingActions(selectedAccountId);
      setStatus(`sync complete: ${summary.folders} folders / ${summary.messages} messages`);
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
        const result = await api.executeMailAction({
          action,
          account_id: selectedAccountId,
          message_ids: [selectedMessageId],
          target_folder_id: targetFolderId ?? null
        });
        await refreshFolders(selectedAccountId);
        await refreshMessages(selectedAccountId, selectedFolderId, query);
        await refreshAudits();
        await refreshPendingActions(selectedAccountId);
        setStatus(result.kind === "pending" ? `action queued: ${actionLabels[action]}` : `action executed: ${actionLabels[action]}`);
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
      refreshPendingActions,
      selectedAccountId,
      selectedFolderId,
      selectedMessageId
    ]
  );

  const handleAccountCreated = useCallback(
    async (account: MailAccount) => {
      setAccounts((current) => [account, ...current.filter((item) => item.id !== account.id)]);
      setSelectedAccountId(account.id);
      setAccountModalOpen(false);
      try {
        await api.syncAccount(account.id);
      } catch (error) {
        setStatus(`initial sync failed: ${String(error)}`);
      }
      await refreshFolders(account.id);
      await refreshMessages(account.id, null, "");
      await refreshSyncState(account.id);
      await refreshAudits();
      await refreshPendingActions(account.id);
      setStatus(`account added and synced: ${account.email}`);
    },
    [refreshAudits, refreshFolders, refreshMessages, refreshPendingActions, refreshSyncState]
  );

  const handleSent = useCallback(
    async (draft: SendMessageDraft) => {
      await api.sendMessage(draft);
      await refreshAudits();
      await refreshPendingActions(draft.account_id);
      setComposerOpen(false);
      setStatus(formatSendQueuedStatus(draft.to));
    },
    [refreshAudits, refreshPendingActions]
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

  const handleConfirmPending = useCallback(
    async (actionId: string) => {
      if (!selectedAccountId || isActionRunning || isAnalyzing) return;
      setActionRunning(true);
      try {
        await api.confirmAction(actionId);
        await refreshFolders(selectedAccountId);
        await refreshMessages(selectedAccountId, selectedFolderId, query);
        await refreshAudits();
        await refreshPendingActions(selectedAccountId);
        setStatus(`pending action confirmed`);
      } catch (error) {
        await refreshAudits();
        await refreshPendingActions(selectedAccountId);
        setStatus(`confirm failed: ${String(error)}`);
      } finally {
        setActionRunning(false);
      }
    },
    [isActionRunning, isAnalyzing, query, refreshAudits, refreshFolders, refreshMessages, refreshPendingActions, selectedAccountId, selectedFolderId]
  );

  const handleRejectPending = useCallback(
    async (actionId: string) => {
      if (!selectedAccountId || isActionRunning || isAnalyzing) return;
      setActionRunning(true);
      try {
        await api.rejectAction(actionId);
        await refreshAudits();
        await refreshPendingActions(selectedAccountId);
        setStatus(`pending action rejected`);
      } catch (error) {
        await refreshAudits();
        await refreshPendingActions(selectedAccountId);
        setStatus(`reject failed: ${String(error)}`);
      } finally {
        setActionRunning(false);
      }
    },
    [isActionRunning, isAnalyzing, refreshAudits, refreshPendingActions, selectedAccountId]
  );

  return (
    <main className="app-shell">
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
          <button className="icon-button" type="button" onClick={() => setAccountModalOpen(true)} title="Add account">
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
                onOpenSettings={() => setAiSettingsOpen(true)}
              />
            </>
          ) : (
            <div className="empty-detail">
              <BellDot size={24} />
              Select a message to inspect headers and body.
            </div>
          )}
        </section>
      </section>

      <footer className="status-console">
        <section className="console-panel">
          <header>SYNC & CONNECTIONS</header>
          <div className="console-row">
            <span>ACCOUNT</span>
            <code>{selectedAccount?.email ?? "none"}</code>
          </div>
          <div className="console-row">
            <span>STATUS</span>
            <code>{accountSyncState?.state ?? "idle"}</code>
          </div>
          <div className="console-row">
            <span>LAST UID</span>
            <code>{accountSyncState?.last_uid ?? "local"}</code>
          </div>
          <div className="console-row">
            <span>FAILURES</span>
            <code>{accountSyncState?.failure_count ?? 0}</code>
          </div>
          <button type="button" onClick={handleSync} disabled={!selectedAccountId}>
            TEST / SYNC ACCOUNT
          </button>
        </section>
        <PendingActionQueue
          actions={pendingActions}
          isActionRunning={isActionRunning || isAnalyzing}
          onConfirm={handleConfirmPending}
          onReject={handleRejectPending}
        />
        <section className="console-panel audit-feed">
          <header>AUDIT / ACTIVITY LOG</header>
          <p>{accountSyncState?.error_message ?? status}</p>
          {audits.slice(0, 4).map((audit) => (
            <code key={audit.id}>
              [{formatTime(audit.created_at)}] {actionLabels[audit.action] ?? audit.action}:{audit.status}
            </code>
          ))}
        </section>
      </footer>

      {isAccountModalOpen ? <AccountModal onClose={() => setAccountModalOpen(false)} onCreated={handleAccountCreated} /> : null}
      {isComposerOpen && selectedAccount ? <Composer account={selectedAccount} onClose={() => setComposerOpen(false)} onSent={handleSent} /> : null}
      {isAiSettingsOpen ? (
        <AiSettingsModal settings={aiSettings} onClose={() => setAiSettingsOpen(false)} onSaved={refreshAiSettings} />
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
  onOpenSettings: () => void;
}

function AiPanel({ settings, insights, status, isAnalyzing, isActionRunning, onAnalyze, onOpenSettings }: AiPanelProps) {
  const latest = insights[0] ?? null;
  const keyStatus = settings?.api_key_mask ? settings.api_key_mask : "not set";

  return (
    <aside className="ai-panel">
      <header>
        <div>
          <PanelRight size={15} />
          <span>AI</span>
        </div>
        <button type="button" onClick={onOpenSettings} title="AI settings">
          <Settings size={14} />
        </button>
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

interface AiSettingsModalProps {
  settings: AiSettingsView | null;
  onClose: () => void;
  onSaved: () => Promise<void>;
}

function AiSettingsModal({ settings, onClose, onSaved }: AiSettingsModalProps) {
  const [form, setForm] = useState<SaveAiSettingsRequest>({
    provider_name: settings?.provider_name ?? "openai-compatible",
    base_url: settings?.base_url ?? "https://api.openai.com/v1",
    model: settings?.model ?? "",
    api_key: "",
    enabled: settings?.enabled ?? false
  });
  const [status, setStatus] = useState(settings?.api_key_mask ? `key saved: ${settings.api_key_mask}` : "key not set");
  const [isSaving, setSaving] = useState(false);

  const update = (field: keyof SaveAiSettingsRequest, value: string | boolean) => {
    setForm((current) => ({ ...current, [field]: value }));
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setSaving(true);
    setStatus("saving ai settings");
    try {
      await api.saveAiSettings({
        ...form,
        api_key: form.api_key?.trim() ? form.api_key : null
      });
      await onSaved();
      onClose();
    } catch (error) {
      setStatus(`save failed: ${String(error)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <form className="modal-panel ai-settings-form" onSubmit={submit}>
        <header>
          <h2>AI SETTINGS</h2>
          <button className="icon-button" type="button" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </header>
        <label>
          Provider
          <input value={form.provider_name} onChange={(event) => update("provider_name", event.target.value)} required />
        </label>
        <label>
          Base URL
          <input value={form.base_url} onChange={(event) => update("base_url", event.target.value)} required />
        </label>
        <label>
          Model
          <input value={form.model} onChange={(event) => update("model", event.target.value)} required />
        </label>
        <label>
          API Key
          <input
            type="password"
            value={form.api_key ?? ""}
            onChange={(event) => update("api_key", event.target.value)}
            placeholder={settings?.api_key_mask ?? "sk-..."}
          />
        </label>
        <label className="checkbox-row">
          <input type="checkbox" checked={form.enabled} onChange={(event) => update("enabled", event.target.checked)} />
          Enabled
        </label>
        <div className="modal-status">{status}</div>
        <footer>
          <button type="button" onClick={onClose}>
            CANCEL
          </button>
          <button type="submit" disabled={isSaving}>
            {isSaving ? "SAVING" : "SAVE"}
          </button>
        </footer>
      </form>
    </div>
  );
}

interface AccountModalProps {
  onClose: () => void;
  onCreated: (account: MailAccount) => Promise<void>;
}

interface PendingActionQueueProps {
  actions: PendingMailAction[];
  isActionRunning: boolean;
  onConfirm: (actionId: string) => Promise<void>;
  onReject: (actionId: string) => Promise<void>;
}

function PendingActionQueue({ actions, isActionRunning, onConfirm, onReject }: PendingActionQueueProps) {
  return (
    <section className="console-panel pending-feed">
      <header>PENDING ACTIONS</header>
      {actions.length === 0 ? <p>queue empty</p> : null}
      {actions.slice(0, 3).map((action) => (
        <div className="pending-action-row" key={action.id}>
          <code>{actionLabels[action.action] ?? action.action}</code>
          <span>{action.draft?.subject ?? `${action.message_ids.length} MSG`}</span>
          <div>
            <button type="button" onClick={() => onConfirm(action.id)} disabled={isActionRunning} title="Confirm action">
              <CheckCheck size={13} />
            </button>
            <button type="button" onClick={() => onReject(action.id)} disabled={isActionRunning} title="Reject action">
              <X size={13} />
            </button>
          </div>
        </div>
      ))}
    </section>
  );
}

function AccountModal({ onClose, onCreated }: AccountModalProps) {
  const [form, setForm] = useState<AddAccountRequest>(defaultAccountForm);
  const [testResult, setTestResult] = useState<string>("not tested");
  const [isSaving, setSaving] = useState(false);

  const update = (field: keyof AddAccountRequest, value: string | number | boolean) => {
    setForm((current) => ({ ...current, [field]: value }));
  };

  const testConnection = async () => {
    try {
      const result = await api.testAccountConnection(form);
      setTestResult(`${result.imap_ok ? "IMAP OK" : "IMAP FAIL"} / ${result.smtp_ok ? "SMTP OK" : "SMTP FAIL"} / ${result.message}`);
    } catch (error) {
      setTestResult(`TEST FAILED / ${String(error)}`);
    }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setSaving(true);
    try {
      const account = await api.addAccount(form);
      await onCreated(account);
    } catch (error) {
      setTestResult(`SAVE FAILED / ${String(error)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <form className="modal-panel" onSubmit={submit}>
        <header>
          <h2>ACCOUNT LINK</h2>
          <button className="icon-button" type="button" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </header>
        <div className="form-grid">
          <label>
            Display
            <input value={form.display_name} onChange={(event) => update("display_name", event.target.value)} required />
          </label>
          <label>
            Email
            <input type="email" value={form.email} onChange={(event) => update("email", event.target.value)} required />
          </label>
          <label>
            Password
            <input type="password" value={form.password} onChange={(event) => update("password", event.target.value)} required />
          </label>
          <label>
            IMAP Host
            <input value={form.imap_host} onChange={(event) => update("imap_host", event.target.value)} required />
          </label>
          <label>
            IMAP Port
            <input type="number" value={form.imap_port} onChange={(event) => update("imap_port", Number(event.target.value))} required />
          </label>
          <label>
            SMTP Host
            <input value={form.smtp_host} onChange={(event) => update("smtp_host", event.target.value)} required />
          </label>
          <label>
            SMTP Port
            <input type="number" value={form.smtp_port} onChange={(event) => update("smtp_port", Number(event.target.value))} required />
          </label>
        </div>
        <div className="modal-status">{testResult}</div>
        <footer>
          <button type="button" onClick={testConnection}>
            TEST
          </button>
          <button type="submit" disabled={isSaving}>
            {isSaving ? "LINKING" : "SAVE"}
          </button>
        </footer>
      </form>
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
