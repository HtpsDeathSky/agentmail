import { describe, expect, it, vi } from "vitest";
import { formatAuditLine, formatFolderCount, formatSendQueuedStatus, refreshAfterMailSyncEvent, runInitialAccountSync } from "./App";

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

describe("runInitialAccountSync", () => {
  it("syncs a saved account, refreshes observable state, starts watchers, and returns a useful status", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
    const refreshPendingActions = vi.fn().mockResolvedValue(undefined);
    const startAccountWatchers = vi.fn().mockResolvedValue(undefined);

    const status = await runInitialAccountSync({
      accountId: "acct-1",
      email: "ops@example.com",
      folderId: "previous-folder",
      query: "release",
      syncAccount: vi.fn().mockResolvedValue({
        account_id: "acct-1",
        folders: 4,
        messages: 18,
        synced_at: "2026-04-27T00:00:00Z"
      }),
      startAccountWatchers,
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits,
      refreshPendingActions
    });

    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", "previous-folder", "release");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(refreshPendingActions).toHaveBeenCalledWith("acct-1");
    expect(startAccountWatchers).toHaveBeenCalledWith("acct-1");
    expect(status).toBe("account saved and initial sync complete: ops@example.com / 4 folders / 18 messages");
  });

  it("keeps the saved account path alive when watcher startup fails after a successful sync", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
    const refreshPendingActions = vi.fn().mockResolvedValue(undefined);
    const startAccountWatchers = vi.fn().mockRejectedValue(new Error("watch unavailable"));

    const status = await runInitialAccountSync({
      accountId: "acct-1",
      email: "ops@example.com",
      folderId: null,
      query: "",
      syncAccount: vi.fn().mockResolvedValue({
        account_id: "acct-1",
        folders: 4,
        messages: 18,
        synced_at: "2026-04-27T00:00:00Z"
      }),
      startAccountWatchers,
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits,
      refreshPendingActions
    });

    expect(startAccountWatchers).toHaveBeenCalledWith("acct-1");
    expect(status).toBe("account saved and initial sync complete: ops@example.com / 4 folders / 18 messages");
  });

  it("keeps the saved account path alive when initial sync fails and skips watcher startup", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
    const refreshPendingActions = vi.fn().mockResolvedValue(undefined);
    const startAccountWatchers = vi.fn().mockResolvedValue(undefined);

    const status = await runInitialAccountSync({
      accountId: "acct-1",
      email: "ops@example.com",
      folderId: null,
      query: "",
      syncAccount: vi.fn().mockRejectedValue(new Error("IMAP login rejected")),
      startAccountWatchers,
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits,
      refreshPendingActions
    });

    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", null, "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(refreshPendingActions).toHaveBeenCalledWith("acct-1");
    expect(startAccountWatchers).not.toHaveBeenCalled();
    expect(status).toBe("account saved, but initial sync failed: Error: IMAP login rejected");
  });

  it("does not sync or start watchers when a saved account has sync disabled", async () => {
    const syncAccount = vi.fn().mockResolvedValue({
      account_id: "acct-1",
      folders: 4,
      messages: 18,
      synced_at: "2026-04-27T00:00:00Z"
    });
    const startAccountWatchers = vi.fn().mockResolvedValue(undefined);
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
    const refreshPendingActions = vi.fn().mockResolvedValue(undefined);

    const status = await runInitialAccountSync({
      accountId: "acct-1",
      email: "ops@example.com",
      syncEnabled: false,
      folderId: null,
      query: "",
      syncAccount,
      startAccountWatchers,
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits,
      refreshPendingActions
    });

    expect(syncAccount).not.toHaveBeenCalled();
    expect(startAccountWatchers).not.toHaveBeenCalled();
    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", null, "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(refreshPendingActions).toHaveBeenCalledWith("acct-1");
    expect(status).toBe("account configuration saved: ops@example.com");
  });
});

describe("refreshAfterMailSyncEvent", () => {
  it("refreshes visible account state after a watcher sync event", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
    const refreshPendingActions = vi.fn().mockResolvedValue(undefined);

    const didRefresh = await refreshAfterMailSyncEvent({
      payload: { account_id: "acct-1", folder_id: "acct-1:inbox", reason: "watch_changed" },
      selectedAccountId: "acct-1",
      selectedFolderId: "acct-1:inbox",
      query: "",
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits,
      refreshPendingActions
    });

    expect(didRefresh).toBe(true);
    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", "acct-1:inbox", "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(refreshPendingActions).toHaveBeenCalledWith("acct-1");
  });

  it("ignores watcher sync events for a different selected account", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
    const refreshPendingActions = vi.fn().mockResolvedValue(undefined);

    const didRefresh = await refreshAfterMailSyncEvent({
      payload: { account_id: "acct-2", folder_id: "acct-2:inbox", reason: "watch_changed" },
      selectedAccountId: "acct-1",
      selectedFolderId: "acct-1:inbox",
      query: "",
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits,
      refreshPendingActions
    });

    expect(didRefresh).toBe(false);
    expect(refreshFolders).not.toHaveBeenCalled();
    expect(refreshMessages).not.toHaveBeenCalled();
    expect(refreshSyncState).not.toHaveBeenCalled();
    expect(refreshAudits).not.toHaveBeenCalled();
    expect(refreshPendingActions).not.toHaveBeenCalled();
  });
});
