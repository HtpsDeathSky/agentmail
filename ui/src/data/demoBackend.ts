import type {
  AccountConfigView,
  AddAccountRequest,
  AiInsight,
  AiPriority,
  AiSettingsView,
  ConnectionTestResult,
  MailAccount,
  MailActionAudit,
  MailActionResult,
  MailActionKind,
  MailActionRequest,
  MailFolder,
  MailMessage,
  MessageQuery,
  SaveAccountConfigRequest,
  SaveAiSettingsRequest,
  SendMessageDraft,
  SyncState,
  SyncSummary
} from "../api";

interface DemoPendingMailAction {
  id: string;
  account_id: string;
  action: MailActionKind;
  message_ids: string[];
  target_folder_id?: string | null;
  local_message_id?: string | null;
  draft?: SendMessageDraft | null;
  status: "pending" | "accepted" | "rejected" | "executed" | "failed";
  error_message?: string | null;
  created_at: string;
  updated_at: string;
}

const now = () => new Date().toISOString();

const account: MailAccount = {
  id: "demo-account",
  display_name: "Operations Mail",
  email: "ops@example.com",
  imap_host: "imap.example.com",
  imap_port: 993,
  imap_tls: true,
  smtp_host: "smtp.example.com",
  smtp_port: 465,
  smtp_tls: true,
  sync_enabled: true,
  created_at: now(),
  updated_at: now()
};

let folders: MailFolder[] = [
  { id: "demo-account:inbox", account_id: account.id, name: "INBOX", path: "INBOX", role: "inbox", unread_count: 3, total_count: 12 },
  { id: "demo-account:sent", account_id: account.id, name: "Sent", path: "Sent", role: "sent", unread_count: 0, total_count: 4 },
  { id: "demo-account:archive", account_id: account.id, name: "Archive", path: "Archive", role: "archive", unread_count: 0, total_count: 32 },
  { id: "demo-account:drafts", account_id: account.id, name: "Drafts", path: "Drafts", role: "drafts", unread_count: 0, total_count: 1 },
  { id: "demo-account:trash", account_id: account.id, name: "Trash", path: "Trash", role: "trash", unread_count: 0, total_count: 0 }
];

let messages: MailMessage[] = [
  {
    id: "msg-001",
    account_id: account.id,
    folder_id: folders[0].id,
    uid: "1001",
    message_id_header: "<1001@agentmail.local>",
    subject: "Security rotation window / action required",
    sender: "infra-watch@example.net",
    recipients: [account.email],
    cc: [],
    received_at: now(),
    body_preview: "Credential rotation window opens tonight. Confirm service owners and blackout exceptions before 18:00.",
    body: "Credential rotation window opens tonight. Confirm service owners and blackout exceptions before 18:00.\n\nThe initial desktop MVP keeps all AI pipelines disabled until the base mail engine is stable.",
    attachments: [],
    flags: { is_read: false, is_starred: true, is_answered: false, is_forwarded: false },
    size_bytes: 4096,
    deleted_at: null
  },
  {
    id: "msg-002",
    account_id: account.id,
    folder_id: folders[0].id,
    uid: "1002",
    message_id_header: "<1002@agentmail.local>",
    subject: "Vendor invoice reconciliation",
    sender: "finance-ops@example.com",
    recipients: [account.email],
    cc: ["audit@example.com"],
    received_at: now(),
    body_preview: "Three invoices are waiting for reconciliation. Attachment metadata is indexed but files are not downloaded yet.",
    body: "Three invoices are waiting for reconciliation. Attachment metadata is indexed but files are not downloaded yet.\n\nFuture local sensitivity review will block remote AI upload for financial and legal material by default.",
    attachments: [
      { id: "att-001", message_id: "msg-002", filename: "invoice-pack.zip", mime_type: "application/zip", size_bytes: 2489000, local_path: null }
    ],
    flags: { is_read: false, is_starred: false, is_answered: false, is_forwarded: false },
    size_bytes: 2491200,
    deleted_at: null
  },
  {
    id: "msg-003",
    account_id: account.id,
    folder_id: folders[0].id,
    uid: "1003",
    message_id_header: "<1003@agentmail.local>",
    subject: "Release train notes",
    sender: "release-control@example.org",
    recipients: [account.email],
    cc: [],
    received_at: now(),
    body_preview: "Build 42 passed smoke tests. Mail client telemetry remains disabled in this MVP.",
    body: "Build 42 passed smoke tests. Mail client telemetry remains disabled in this MVP.\n\nNext backend cut should replace the mock protocol with live IMAP/SMTP adapters.",
    attachments: [],
    flags: { is_read: true, is_starred: false, is_answered: false, is_forwarded: false },
    size_bytes: 3068,
    deleted_at: null
  },
  {
    id: "msg-101",
    account_id: account.id,
    folder_id: folders[1].id,
    uid: "2001",
    message_id_header: "<2001@agentmail.local>",
    subject: "Sent: rotation confirmation",
    sender: account.email,
    recipients: ["infra-watch@example.net"],
    cc: [],
    received_at: now(),
    body_preview: "Confirmation sent through the pending action queue.",
    body: "Confirmation sent through the pending action queue.\n\nThis row exercises non-INBOX folder navigation in the browser demo.",
    attachments: [],
    flags: { is_read: true, is_starred: false, is_answered: false, is_forwarded: false },
    size_bytes: 2048,
    deleted_at: null
  },
  {
    id: "msg-201",
    account_id: account.id,
    folder_id: folders[2].id,
    uid: "3001",
    message_id_header: "<3001@agentmail.local>",
    subject: "Archived vendor thread",
    sender: "finance-ops@example.com",
    recipients: [account.email],
    cc: [],
    received_at: now(),
    body_preview: "Archived reconciliation notes remain searchable outside INBOX.",
    body: "Archived reconciliation notes remain searchable outside INBOX.",
    attachments: [],
    flags: { is_read: true, is_starred: false, is_answered: false, is_forwarded: false },
    size_bytes: 2048,
    deleted_at: null
  },
  {
    id: "msg-301",
    account_id: account.id,
    folder_id: folders[3].id,
    uid: "4001",
    message_id_header: "<4001@agentmail.local>",
    subject: "Draft: incident summary",
    sender: account.email,
    recipients: ["ops-lead@example.com"],
    cc: [],
    received_at: now(),
    body_preview: "Draft content is indexed for folder navigation validation.",
    body: "Draft content is indexed for folder navigation validation.",
    attachments: [],
    flags: { is_read: true, is_starred: false, is_answered: false, is_forwarded: false },
    size_bytes: 1024,
    deleted_at: null
  },
  {
    id: "msg-401",
    account_id: account.id,
    folder_id: folders[4].id,
    uid: "5001",
    message_id_header: "<5001@agentmail.local>",
    subject: "Deleted obsolete alert",
    sender: "alerts@example.net",
    recipients: [account.email],
    cc: [],
    received_at: now(),
    body_preview: "Trash folder sync proves delete-to-trash can be inspected.",
    body: "Trash folder sync proves delete-to-trash can be inspected.",
    attachments: [],
    flags: { is_read: true, is_starred: false, is_answered: false, is_forwarded: false },
    size_bytes: 1024,
    deleted_at: null
  }
];

let audits: MailActionAudit[] = [
  {
    id: "audit-001",
    account_id: account.id,
    action: "mark_read",
    message_ids: [],
    status: "executed",
    created_at: now(),
    error_message: null
  }
];

let accounts = [account];
let accountPasswords: Record<string, string> = {
  [account.id]: "demo-mail-secret"
};
let pendingActions: DemoPendingMailAction[] = [];
let aiSettings: AiSettingsView | null = {
  provider_name: "openai-compatible",
  base_url: "https://api.example.com/v1",
  model: "demo-mail-model",
  enabled: true,
  api_key_mask: "sk-...demo"
};
let aiInsights: AiInsight[] = [];

const recordAudit = (
  action: MailActionAudit["action"],
  accountId: string,
  messageIds: string[],
  status: MailActionAudit["status"] = "executed"
) => {
  audits = [
    {
      id: crypto.randomUUID(),
      account_id: accountId,
      action,
      message_ids: messageIds,
      status,
      error_message: null,
      created_at: now()
    },
    ...audits
  ];
};

const actionResult = (kind: MailActionResult["kind"], pendingActionId?: string): MailActionResult => ({
  kind,
  pending_action_id: pendingActionId ?? null
});

const maskApiKey = (apiKey: string) => {
  const codePoints = Array.from(apiKey.trim());
  if (codePoints.length <= 8) return "****";
  return `${codePoints.slice(0, 3).join("")}...${codePoints.slice(-4).join("")}`;
};

const validateAiBaseUrl = (baseUrl: string) => {
  let parsed: URL;
  try {
    parsed = new URL(baseUrl.trim());
  } catch {
    throw new Error("base_url must be a valid https URL");
  }
  if (parsed.protocol !== "https:") throw new Error("base_url must use https");
};

const isSentFolderPath = (path: string) => {
  const parts = path.toLowerCase().split(/[/.]/);
  const name = parts[parts.length - 1] ?? "";
  return ["sent", "sent mail", "sent messages", "sent items"].includes(name);
};

export const demoBackend = {
  async invoke(command: string, args?: Record<string, unknown>): Promise<unknown> {
    await new Promise((resolve) => window.setTimeout(resolve, 120));

    switch (command) {
      case "add_account": {
        const request = args?.request as AddAccountRequest;
        const next: MailAccount = {
          ...account,
          id: crypto.randomUUID(),
          display_name: request.display_name,
          email: request.email,
          imap_host: request.imap_host,
          imap_port: request.imap_port,
          imap_tls: request.imap_tls,
          smtp_host: request.smtp_host,
          smtp_port: request.smtp_port,
          smtp_tls: request.smtp_tls,
          created_at: now(),
          updated_at: now()
        };
        accounts = [next, ...accounts];
        accountPasswords[next.id] = request.password;
        recordAudit("mark_read", next.id, []);
        return next;
      }
      case "get_account_config": {
        const accountId = (args?.accountId ?? args?.account_id) as string;
        const found = accounts.find((item) => item.id === accountId);
        if (!found) throw new Error("account not found");
        const config: AccountConfigView = {
          ...found,
          password: accountPasswords[found.id] ?? ""
        };
        return config;
      }
      case "save_account_config": {
        const request = args?.request as SaveAccountConfigRequest;
        const existing = request.id ? accounts.find((item) => item.id === request.id) : null;
        const next: MailAccount = {
          id: existing?.id ?? crypto.randomUUID(),
          display_name: request.display_name,
          email: request.email,
          imap_host: request.imap_host,
          imap_port: request.imap_port,
          imap_tls: request.imap_tls,
          smtp_host: request.smtp_host,
          smtp_port: request.smtp_port,
          smtp_tls: request.smtp_tls,
          sync_enabled: request.sync_enabled,
          created_at: existing?.created_at ?? now(),
          updated_at: now()
        };
        accounts = [next, ...accounts.filter((item) => item.id !== next.id)];
        accountPasswords[next.id] = request.password;
        recordAudit("mark_read", next.id, []);
        return next;
      }
      case "test_account_connection": {
        const result: ConnectionTestResult = {
          imap_ok: true,
          smtp_ok: true,
          message: "demo runtime accepted account settings"
        };
        return result;
      }
      case "list_accounts":
        return accounts;
      case "sync_account": {
        const accountId = args?.accountId as string;
        const summary: SyncSummary = {
          account_id: accountId,
          folders: folders.length,
          messages: messages.filter((message) => message.account_id === account.id).length,
          last_uid: "1003",
          synced_at: now()
        };
        recordAudit("mark_read", accountId, []);
        return summary;
      }
      case "start_account_watchers":
        return null;
      case "get_sync_status": {
        const accountId = args?.accountId as string;
        const state: SyncState[] = [
          {
            account_id: accountId,
            folder_id: null,
            state: "idle",
            last_uid: "1003",
            last_synced_at: now(),
            error_message: null,
            backoff_until: null,
            failure_count: 0
          }
        ];
        return state;
      }
      case "list_folders":
        return folders;
      case "list_messages": {
        const query = args?.query as MessageQuery;
        return messages
          .filter((message) => !message.deleted_at)
          .filter((message) => !query.account_id || message.account_id === query.account_id)
          .filter((message) => !query.folder_id || message.folder_id === query.folder_id)
          .slice(query.offset, query.offset + query.limit);
      }
      case "get_message":
        return messages.find((message) => message.id === args?.messageId) ?? messages[0];
      case "search_messages": {
        const term = String(args?.term ?? "").toLowerCase();
        return messages.filter((message) =>
          [message.subject, message.sender, message.body_preview, message.body ?? ""].some((value) => value.toLowerCase().includes(term))
        );
      }
      case "execute_mail_action": {
        const request = args?.request as MailActionRequest;
        if (
          request.action === "send" ||
          request.action === "forward" ||
          request.action === "permanent_delete" ||
          request.action === "batch_delete" ||
          request.action === "batch_move"
        ) {
          const pending: DemoPendingMailAction = {
            id: crypto.randomUUID(),
            account_id: request.account_id,
            action: request.action,
            message_ids: request.message_ids,
            target_folder_id: request.target_folder_id ?? null,
            draft: null,
            status: "pending",
            error_message: null,
            created_at: now(),
            updated_at: now()
          };
          pendingActions = [pending, ...pendingActions];
          recordAudit(request.action, request.account_id, request.message_ids, "queued");
          return actionResult("pending", pending.id);
        }
        messages = messages.map((message) => {
          if (!request.message_ids.includes(message.id)) return message;
          if (request.action === "mark_read") return { ...message, flags: { ...message.flags, is_read: true } };
          if (request.action === "mark_unread") return { ...message, flags: { ...message.flags, is_read: false } };
          if (request.action === "star") return { ...message, flags: { ...message.flags, is_starred: true } };
          if (request.action === "unstar") return { ...message, flags: { ...message.flags, is_starred: false } };
          if (request.action === "delete") return { ...message, folder_id: folders.find((folder) => folder.role === "trash")?.id ?? message.folder_id, uid: null };
          if ((request.action === "move" || request.action === "archive") && request.target_folder_id) {
            return { ...message, folder_id: request.target_folder_id, uid: null };
          }
          if (request.action === "archive") {
            return { ...message, folder_id: folders.find((folder) => folder.role === "archive")?.id ?? message.folder_id, uid: null };
          }
          return message;
        });
        recordAudit(request.action, request.account_id, request.message_ids);
        return actionResult("executed");
      }
      case "send_message": {
        const incomingDraft = args?.draft as SendMessageDraft;
        const draftAccount = accounts.find((item) => item.id === incomingDraft.account_id);
        if (!draftAccount) throw new Error("account not found");
        let sentFolder =
          folders.find((folder) => folder.account_id === incomingDraft.account_id && folder.role === "sent") ??
          folders.find((folder) => folder.account_id === incomingDraft.account_id && isSentFolderPath(folder.path));
        if (!sentFolder) {
          sentFolder = {
            id: `${incomingDraft.account_id}:sent`,
            account_id: incomingDraft.account_id,
            name: "Sent",
            path: "Sent",
            role: "sent",
            unread_count: 0,
            total_count: 0
          };
          folders = [...folders, sentFolder];
        } else {
          sentFolder.role = "sent";
        }
        const messageId = crypto.randomUUID();
        const messageIdHeader = incomingDraft.message_id_header ?? `<${messageId}@agentmail.local>`;
        const draft: SendMessageDraft = { ...incomingDraft, message_id_header: messageIdHeader };
        const pending: DemoPendingMailAction = {
          id: crypto.randomUUID(),
          account_id: draft.account_id,
          action: "send",
          message_ids: [],
          target_folder_id: null,
          local_message_id: messageId,
          draft,
          status: "pending",
          error_message: null,
          created_at: now(),
          updated_at: now()
        };
        pendingActions = [pending, ...pendingActions];
        const bodyPreview = draft.body.trim() || "(empty message)";
        messages = [
          {
            id: messageId,
            account_id: draft.account_id,
            folder_id: sentFolder.id,
            uid: null,
            message_id_header: messageIdHeader,
            subject: draft.subject,
            sender: draftAccount.email,
            recipients: draft.to,
            cc: draft.cc,
            received_at: now(),
            body_preview: bodyPreview.slice(0, 180),
            body: draft.body,
            attachments: [],
            flags: { is_read: true, is_starred: false, is_answered: true, is_forwarded: false },
            size_bytes: null,
            deleted_at: null
          },
          ...messages
        ];
        sentFolder.total_count = messages.filter((message) => message.folder_id === sentFolder.id && !message.deleted_at).length;
        sentFolder.unread_count = messages.filter((message) => message.folder_id === sentFolder.id && !message.deleted_at && !message.flags.is_read).length;
        recordAudit("send", draft.account_id, [], "queued");
        return pending.id;
      }
      case "get_audit_log":
        return audits;
      case "list_pending_actions": {
        const accountId = (args?.accountId ?? args?.account_id) as string | null | undefined;
        return pendingActions.filter((action) => action.status === "pending").filter((action) => !accountId || action.account_id === accountId);
      }
      case "confirm_action": {
        const actionId = (args?.actionId ?? args?.action_id) as string;
        const pending = pendingActions.find((action) => action.id === actionId);
        if (!pending) throw new Error("pending action not found");
        pending.status = "executed";
        pending.updated_at = now();
        recordAudit(pending.action, pending.account_id, pending.message_ids, "accepted");
        recordAudit(pending.action, pending.account_id, pending.message_ids, "executed");
        return actionResult("executed");
      }
      case "reject_action": {
        const actionId = (args?.actionId ?? args?.action_id) as string;
        const pending = pendingActions.find((action) => action.id === actionId);
        if (!pending) throw new Error("pending action not found");
        pending.status = "rejected";
        pending.updated_at = now();
        if (pending.action === "send" && pending.local_message_id) {
          const placeholder = messages.find((message) => message.id === pending.local_message_id);
          const folder = placeholder ? folders.find((item) => item.id === placeholder.folder_id) : null;
          if (
            placeholder &&
            folder?.role === "sent" &&
            placeholder.account_id === pending.account_id &&
            placeholder.uid == null &&
            placeholder.message_id_header === pending.draft?.message_id_header
          ) {
            placeholder.deleted_at = now();
            folder.total_count = messages.filter((message) => message.folder_id === folder.id && !message.deleted_at).length;
            folder.unread_count = messages.filter((message) => message.folder_id === folder.id && !message.deleted_at && !message.flags.is_read).length;
          }
        }
        recordAudit(pending.action, pending.account_id, pending.message_ids, "rejected");
        return null;
      }
      case "get_ai_settings":
        return aiSettings;
      case "save_ai_settings": {
        const request = args?.request as SaveAiSettingsRequest;
        validateAiBaseUrl(request.base_url);
        const apiKey = request.api_key?.trim() ?? "";
        const apiKeyMask = apiKey ? maskApiKey(apiKey) : aiSettings?.api_key_mask;
        if (!apiKeyMask) throw new Error("api_key is required");
        aiSettings = {
          provider_name: request.provider_name,
          base_url: request.base_url,
          model: request.model,
          enabled: request.enabled,
          api_key_mask: apiKeyMask
        };
        return aiSettings;
      }
      case "clear_ai_settings":
        aiSettings = null;
        return null;
      case "run_ai_analysis": {
        if (!aiSettings || !aiSettings.enabled || !aiSettings.api_key_mask) throw new Error("AI settings are missing or disabled");
        const messageId = args?.messageId as string;
        const message = messages.find((candidate) => candidate.id === messageId);
        if (!message) throw new Error("message not found");

        const hasAction = /\b(action|required|confirm|waiting|before|tonight)\b/i.test(`${message.subject} ${message.body_preview}`);
        const priority: AiPriority = hasAction ? "high" : "normal";
        const summary = hasAction
          ? "需要在截止时间前确认相关负责人和例外事项。"
          : message.attachments.length > 0
            ? "邮件包含待核对附件，请按需查看处理。"
            : "邮件内容已整理，可按需阅读归档。";
        const payload = {
          summary,
          category: message.attachments.length > 0 ? "财务" : hasAction ? "运维" : "一般",
          priority,
          todos: hasAction ? ["确认邮件要求的事项并及时回复。"] : [],
          reply_draft: hasAction ? "已收到，我会在截止时间前确认并回复。" : ""
        };
        const insight: AiInsight = {
          id: `demo-ai-${message.id}-${aiInsights.filter((item) => item.message_id === message.id).length + 1}`,
          message_id: message.id,
          provider_name: aiSettings.provider_name,
          model: aiSettings.model,
          ...payload,
          raw_json: JSON.stringify(payload),
          created_at: now()
        };
        aiInsights = [insight, ...aiInsights];
        return insight;
      }
      case "list_ai_insights": {
        const messageId = args?.messageId as string;
        return aiInsights.filter((insight) => insight.message_id === messageId);
      }
      default:
        throw new Error(`unknown demo command: ${command}`);
    }
  }
};
