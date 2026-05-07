import type { MailActionKind, MailMessage } from "../api";

export const actionLabels: Record<MailActionKind, string> = {
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

export type ContextMenuActionItem =
  | {
      kind: "action";
      label: string;
      action: MailActionKind;
      disabled: boolean;
    }
  | {
      kind: "analyze";
      label: string;
      disabled: boolean;
    };

export function shouldAutoMarkRead(message: MailMessage | null) {
  return Boolean(message && !message.flags.is_read);
}

export function getContextMenuActionItems(message: MailMessage): ContextMenuActionItem[] {
  return [
    {
      kind: "action",
      label: "READ",
      action: "mark_read",
      disabled: message.flags.is_read
    },
    {
      kind: "action",
      label: message.flags.is_starred ? "UNSTAR" : "STAR",
      action: message.flags.is_starred ? "unstar" : "star",
      disabled: false
    },
    {
      kind: "action",
      label: "DELETE",
      action: "delete",
      disabled: false
    },
    {
      kind: "analyze",
      label: "ANALYZE",
      disabled: false
    }
  ];
}

export function shouldRefreshAiInsightsForAnalyzedMessage(
  analyzedMessageId: string,
  selectedMessageId: string | null
) {
  return analyzedMessageId === selectedMessageId;
}
