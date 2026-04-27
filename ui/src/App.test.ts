import { describe, expect, it } from "vitest";
import { formatAuditLine, formatFolderCount, formatSendQueuedStatus } from "./App";

describe("formatFolderCount", () => {
  it("shows total count when no messages are unread", () => {
    expect(formatFolderCount({ unread_count: 0, total_count: 8 })).toBe("8");
  });

  it("shows unread and total counts when unread messages exist", () => {
    expect(formatFolderCount({ unread_count: 3, total_count: 8 })).toBe("3/8");
  });
});

describe("formatSendQueuedStatus", () => {
  it("states that queued sends need pending action confirmation", () => {
    expect(formatSendQueuedStatus(["ops@example.com"])).toBe(
      "send queued for ops@example.com / confirm SEND in PENDING ACTIONS"
    );
  });
});

describe("formatAuditLine", () => {
  it("keeps failed action error details visible in activity log text", () => {
    expect(
      formatAuditLine({
        id: "audit-1",
        account_id: "acct",
        action: "send",
        message_ids: [],
        status: "failed",
        error_message: "SMTP authentication rejected by provider",
        created_at: "2026-04-27T07:30:00Z"
      })
    ).toContain("SMTP authentication rejected by provider");
  });
});
