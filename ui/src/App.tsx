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
import { FormEvent, useCallback, useEffect, useMemo, useState, useTransition } from "react";
import {
  AddAccountRequest,
  api,
  MailAccount,
  MailActionAudit,
  MailActionKind,
  MailFolder,
  MailMessage,
  PendingMailAction,
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
  const [isPending, startTransition] = useTransition();

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

  useEffect(() => {
    void refreshAccounts().then(refreshAudits).catch((error) => setStatus(`startup failed: ${String(error)}`));
  }, [refreshAccounts, refreshAudits]);

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
    if (!selectedMessageId) {
      setSelectedMessage(null);
      return;
    }
    void api.getMessage(selectedMessageId).then(setSelectedMessage).catch(() => {
      setSelectedMessage(messages.find((message) => message.id === selectedMessageId) ?? null);
    });
  }, [messages, selectedMessageId]);

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
      if (!selectedAccountId || !selectedMessageId) return;
      const result = await api.executeMailAction({
        action,
        account_id: selectedAccountId,
        message_ids: [selectedMessageId],
        target_folder_id: targetFolderId ?? null
      });
      await refreshMessages(selectedAccountId, selectedFolderId, query);
      await refreshAudits();
      await refreshPendingActions(selectedAccountId);
      setStatus(result.kind === "pending" ? `action queued: ${actionLabels[action]}` : `action executed: ${actionLabels[action]}`);
    },
    [query, refreshAudits, refreshMessages, refreshPendingActions, selectedAccountId, selectedFolderId, selectedMessageId]
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
      setStatus(`send queued for ${draft.to.join(", ")}`);
    },
    [refreshAudits, refreshPendingActions]
  );

  const handleConfirmPending = useCallback(
    async (actionId: string) => {
      if (!selectedAccountId) return;
      try {
        await api.confirmAction(actionId);
        await refreshMessages(selectedAccountId, selectedFolderId, query);
        await refreshAudits();
        await refreshPendingActions(selectedAccountId);
        setStatus(`pending action confirmed`);
      } catch (error) {
        await refreshAudits();
        await refreshPendingActions(selectedAccountId);
        setStatus(`confirm failed: ${String(error)}`);
      }
    },
    [query, refreshAudits, refreshMessages, refreshPendingActions, selectedAccountId, selectedFolderId]
  );

  const handleRejectPending = useCallback(
    async (actionId: string) => {
      if (!selectedAccountId) return;
      await api.rejectAction(actionId);
      await refreshAudits();
      await refreshPendingActions(selectedAccountId);
      setStatus(`pending action rejected`);
    },
    [refreshAudits, refreshPendingActions, selectedAccountId]
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
                  <code>{folder.unread_count}</code>
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
              AI OFFLINE
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
                <button type="button" onClick={() => runAction(selectedMessage.flags.is_read ? "mark_unread" : "mark_read")}>
                  <CheckCheck size={15} />
                  {selectedMessage.flags.is_read ? "UNREAD" : "READ"}
                </button>
                <button type="button" onClick={() => runAction(selectedMessage.flags.is_starred ? "unstar" : "star")}>
                  <Star size={15} />
                  {selectedMessage.flags.is_starred ? "UNSTAR" : "STAR"}
                </button>
                <button type="button" onClick={() => runAction("delete")}>
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
              <aside className="ai-placeholder">
                <div>
                  <PanelRight size={15} />
                  AI PIPELINE RESERVED
                </div>
                <p>Local model review and remote summary are intentionally disabled in MVP.</p>
              </aside>
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
        <PendingActionQueue actions={pendingActions} onConfirm={handleConfirmPending} onReject={handleRejectPending} />
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
    </main>
  );
}

interface AccountModalProps {
  onClose: () => void;
  onCreated: (account: MailAccount) => Promise<void>;
}

interface PendingActionQueueProps {
  actions: PendingMailAction[];
  onConfirm: (actionId: string) => Promise<void>;
  onReject: (actionId: string) => Promise<void>;
}

function PendingActionQueue({ actions, onConfirm, onReject }: PendingActionQueueProps) {
  return (
    <section className="console-panel pending-feed">
      <header>PENDING ACTIONS</header>
      {actions.length === 0 ? <p>queue empty</p> : null}
      {actions.slice(0, 3).map((action) => (
        <div className="pending-action-row" key={action.id}>
          <code>{actionLabels[action.action] ?? action.action}</code>
          <span>{action.draft?.subject ?? `${action.message_ids.length} MSG`}</span>
          <div>
            <button type="button" onClick={() => onConfirm(action.id)} title="Confirm action">
              <CheckCheck size={13} />
            </button>
            <button type="button" onClick={() => onReject(action.id)} title="Reject action">
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
