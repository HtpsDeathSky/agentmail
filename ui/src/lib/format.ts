import { MailActionAudit, MailFolder } from "../api";
import { actionLabels } from "./mailActions";

export function formatFolderCount(folder: Pick<MailFolder, "unread_count" | "total_count">) {
  if (folder.unread_count > 0) return `${folder.unread_count}/${folder.total_count}`;
  return String(folder.total_count);
}

export function formatSendStatus(recipients: string[]) {
  return `sent to ${recipients.join(", ")}`;
}

export function formatAuditLine(audit: MailActionAudit) {
  const base = `[${formatTime(audit.created_at)}] ${actionLabels[audit.action] ?? audit.action}:${audit.status}`;
  return audit.error_message ? `${base} / ${audit.error_message}` : base;
}

export function formatTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "unknown";
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(date);
}

export function formatSize(value?: number | null) {
  if (!value) return "0 B";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}
