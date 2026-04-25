import { invoke } from "@tauri-apps/api/core";
import { demoBackend } from "./data/demoBackend";

export type Timestamp = string;

export interface MailAccount {
  id: string;
  display_name: string;
  email: string;
  imap_host: string;
  imap_port: number;
  imap_tls: boolean;
  smtp_host: string;
  smtp_port: number;
  smtp_tls: boolean;
  sync_enabled: boolean;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export interface MailFolder {
  id: string;
  account_id: string;
  name: string;
  path: string;
  role: "inbox" | "sent" | "archive" | "trash" | "drafts" | "junk" | "custom";
  unread_count: number;
  total_count: number;
}

export interface MessageFlags {
  is_read: boolean;
  is_starred: boolean;
  is_answered: boolean;
  is_forwarded: boolean;
}

export interface AttachmentRef {
  id: string;
  message_id: string;
  filename: string;
  mime_type: string;
  size_bytes: number;
  local_path?: string | null;
}

export interface MailMessage {
  id: string;
  account_id: string;
  folder_id: string;
  uid?: string | null;
  message_id_header?: string | null;
  subject: string;
  sender: string;
  recipients: string[];
  cc: string[];
  received_at: Timestamp;
  body_preview: string;
  body?: string | null;
  attachments: AttachmentRef[];
  flags: MessageFlags;
  size_bytes?: number | null;
  deleted_at?: Timestamp | null;
}

export interface MessageQuery {
  account_id?: string | null;
  folder_id?: string | null;
  limit: number;
  offset: number;
}

export interface AddAccountRequest {
  display_name: string;
  email: string;
  password: string;
  imap_host: string;
  imap_port: number;
  imap_tls: boolean;
  smtp_host: string;
  smtp_port: number;
  smtp_tls: boolean;
}

export interface ConnectionTestResult {
  imap_ok: boolean;
  smtp_ok: boolean;
  message: string;
}

export interface SyncState {
  account_id: string;
  folder_id?: string | null;
  state: "idle" | "syncing" | "watching" | "backoff" | "error" | "disabled";
  last_uid?: string | null;
  last_synced_at?: Timestamp | null;
  error_message?: string | null;
  backoff_until?: Timestamp | null;
  failure_count: number;
}

export interface SyncSummary {
  account_id: string;
  folders: number;
  messages: number;
  last_uid?: string | null;
  synced_at: Timestamp;
}

export type MailActionKind =
  | "mark_read"
  | "mark_unread"
  | "star"
  | "unstar"
  | "move"
  | "archive"
  | "delete"
  | "permanent_delete"
  | "send"
  | "forward"
  | "batch_delete"
  | "batch_move";

export interface MailActionRequest {
  action: MailActionKind;
  account_id: string;
  message_ids: string[];
  target_folder_id?: string | null;
}

export interface MailActionAudit {
  id: string;
  account_id: string;
  action: MailActionKind;
  message_ids: string[];
  status: "queued" | "accepted" | "rejected" | "executed" | "failed";
  error_message?: string | null;
  created_at: Timestamp;
}

export interface MailActionResult {
  kind: "executed" | "pending";
  pending_action_id?: string | null;
}

export interface SendMessageDraft {
  account_id: string;
  to: string[];
  cc: string[];
  subject: string;
  body: string;
}

export interface PendingMailAction {
  id: string;
  account_id: string;
  action: MailActionKind;
  message_ids: string[];
  target_folder_id?: string | null;
  draft?: SendMessageDraft | null;
  status: "pending" | "accepted" | "rejected" | "executed" | "failed";
  error_message?: string | null;
  created_at: Timestamp;
  updated_at: Timestamp;
}

export type AiPriority = "low" | "normal" | "high" | "urgent";

export interface AiSettingsView {
  provider_name: string;
  base_url: string;
  model: string;
  enabled: boolean;
  api_key_mask?: string | null;
}

export interface SaveAiSettingsRequest {
  provider_name: string;
  base_url: string;
  model: string;
  api_key?: string | null;
  enabled: boolean;
}

export interface AiInsight {
  id: string;
  message_id: string;
  provider_name: string;
  model: string;
  summary: string;
  category: string;
  priority: AiPriority;
  todos: string[];
  reply_draft: string;
  raw_json: string;
  created_at: Timestamp;
}

type CommandMap = {
  add_account: MailAccount;
  test_account_connection: ConnectionTestResult;
  list_accounts: MailAccount[];
  sync_account: SyncSummary;
  get_sync_status: SyncState[];
  list_folders: MailFolder[];
  list_messages: MailMessage[];
  get_message: MailMessage;
  search_messages: MailMessage[];
  execute_mail_action: MailActionResult;
  send_message: string;
  get_audit_log: MailActionAudit[];
  list_pending_actions: PendingMailAction[];
  confirm_action: MailActionResult;
  reject_action: null;
  get_ai_settings: AiSettingsView | null;
  save_ai_settings: AiSettingsView;
  clear_ai_settings: null;
  run_ai_analysis: AiInsight;
  list_ai_insights: AiInsight[];
};

const hasTauri = () => Boolean((window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);

async function call<T extends keyof CommandMap>(command: T, args?: Record<string, unknown>): Promise<CommandMap[T]> {
  if (hasTauri()) {
    return invoke<CommandMap[T]>(command, args);
  }
  return demoBackend.invoke(command, args) as Promise<CommandMap[T]>;
}

export const api = {
  addAccount: (request: AddAccountRequest) => call("add_account", { request }),
  testAccountConnection: (request: AddAccountRequest) =>
    call("test_account_connection", { request: { account_id: null, manual: request } }),
  listAccounts: () => call("list_accounts"),
  syncAccount: (accountId: string) => call("sync_account", { accountId }),
  getSyncStatus: (accountId: string) => call("get_sync_status", { accountId }),
  listFolders: (accountId: string) => call("list_folders", { accountId }),
  listMessages: (query: MessageQuery) => call("list_messages", { query }),
  getMessage: (messageId: string) => call("get_message", { messageId }),
  searchMessages: (term: string, limit = 100) => call("search_messages", { term, limit }),
  executeMailAction: (request: MailActionRequest) => call("execute_mail_action", { request }),
  sendMessage: (draft: SendMessageDraft) => call("send_message", { draft }),
  getAuditLog: (limit = 100) => call("get_audit_log", { limit }),
  listPendingActions: (accountId?: string | null) => call("list_pending_actions", { accountId: accountId ?? null }),
  confirmAction: (actionId: string) => call("confirm_action", { actionId }),
  rejectAction: (actionId: string) => call("reject_action", { actionId }),
  getAiSettings: () => call("get_ai_settings"),
  saveAiSettings: (request: SaveAiSettingsRequest) => call("save_ai_settings", { request }),
  clearAiSettings: () => call("clear_ai_settings"),
  runAiAnalysis: (messageId: string) => call("run_ai_analysis", { messageId }),
  listAiInsights: (messageId: string) => call("list_ai_insights", { messageId })
};
