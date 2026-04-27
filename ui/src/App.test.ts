import { describe, expect, it } from "vitest";
import { formatFolderCount, formatSendQueuedStatus } from "./App";

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
