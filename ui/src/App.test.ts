import { describe, expect, it, vi } from "vitest";
import {
  applyThemeModeToDocument,
  clampWorkspaceSplitPercent,
  formatAuditLine,
  formatFolderCount,
  formatSendQueuedStatus,
  getWorkspaceSplitModel,
  getNextThemeMode,
  readStoredWorkspaceSplitPercent,
  readStoredThemeMode,
  refreshAfterMailSyncEvent,
  runAutomaticAccountSync,
  runInitialAccountSync,
  WORKSPACE_SPLIT_STORAGE_KEY,
  THEME_MODE_STORAGE_KEY
} from "./App";

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

describe("theme mode helpers", () => {
  it("defaults to dark mode when no saved preference exists", () => {
    window.localStorage.removeItem(THEME_MODE_STORAGE_KEY);

    expect(readStoredThemeMode(window.localStorage)).toBe("dark");
  });

  it("toggles between dark and light modes", () => {
    expect(getNextThemeMode("dark")).toBe("light");
    expect(getNextThemeMode("light")).toBe("dark");
  });

  it("persists and applies light mode to the document root", () => {
    window.localStorage.removeItem(THEME_MODE_STORAGE_KEY);

    applyThemeModeToDocument(document.documentElement, window.localStorage, "light");

    expect(window.localStorage.getItem(THEME_MODE_STORAGE_KEY)).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});

describe("workspace split helpers", () => {
  it("keeps a valid split percentage when both panes satisfy their minimum widths", () => {
    expect(clampWorkspaceSplitPercent(42, 1000, 320, 420)).toBe(42);
  });

  it("clamps the list pane to its minimum width", () => {
    expect(clampWorkspaceSplitPercent(20, 1000, 320, 420)).toBe(32);
  });

  it("clamps the detail pane to its minimum width", () => {
    expect(clampWorkspaceSplitPercent(70, 1000, 320, 420)).toBe(58);
  });

  it("falls back to an even split when the container cannot fit both minimum widths", () => {
    expect(clampWorkspaceSplitPercent(80, 600, 320, 420)).toBe(50);
  });

  it("uses the exact feasible split when the container exactly fits both minimum widths", () => {
    expect(clampWorkspaceSplitPercent(80, 740, 320, 420)).toBeCloseTo(43.24, 2);
  });

  it("reads only valid stored split percentages", () => {
    window.localStorage.setItem(WORKSPACE_SPLIT_STORAGE_KEY, "44.5");

    expect(readStoredWorkspaceSplitPercent(window.localStorage)).toBe(44.5);

    window.localStorage.setItem(WORKSPACE_SPLIT_STORAGE_KEY, "not-a-number");

    expect(readStoredWorkspaceSplitPercent(window.localStorage)).toBe(45);
  });

  it("falls back when storage access throws", () => {
    const storage = {
      getItem: vi.fn(() => {
        throw new Error("blocked");
      })
    };

    expect(readStoredWorkspaceSplitPercent(storage)).toBe(45);
  });

  it("computes a feasible split range from measured workspace width minus the divider", () => {
    expect(getWorkspaceSplitModel(95, 1000, 320, 420)).toEqual({
      percent: 58,
      minPercent: 32,
      maxPercent: 58
    });
  });

  it("uses the clamped split for display when stored input is outside the measured range", () => {
    const model = getWorkspaceSplitModel(10, 1000, 320, 420);

    expect(model.percent).toBe(32);
    expect(Math.round(100 - model.percent)).toBe(68);
  });

  it("uses an honest constrained range when both minimum panes cannot fit", () => {
    expect(getWorkspaceSplitModel(95, 532, 320, 420)).toEqual({
      percent: 50,
      minPercent: 50,
      maxPercent: 50
    });
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

describe("runAutomaticAccountSync", () => {
  it("syncs and refreshes the visible account even when watcher startup is unavailable", async () => {
    const syncAccount = vi.fn().mockResolvedValue({
      account_id: "acct-1",
      folders: 4,
      messages: 19,
      synced_at: "2026-04-27T00:01:00Z"
    });
    const startAccountWatchers = vi.fn().mockRejectedValue(new Error("IDLE unavailable"));
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
    const refreshPendingActions = vi.fn().mockResolvedValue(undefined);

    const result = await runAutomaticAccountSync({
      selectedAccountId: "acct-1",
      selectedFolderId: "acct-1:inbox",
      query: "",
      syncAccount,
      startAccountWatchers,
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits,
      refreshPendingActions
    });

    expect(result).toEqual({ refreshed: true, status: "auto sync complete: 4 folders / 19 messages" });
    expect(syncAccount).toHaveBeenCalledWith("acct-1");
    expect(startAccountWatchers).toHaveBeenCalledWith("acct-1");
    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", "acct-1:inbox", "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(refreshPendingActions).toHaveBeenCalledWith("acct-1");
  });

  it("does nothing when no account is selected", async () => {
    const syncAccount = vi.fn().mockResolvedValue({
      account_id: "acct-1",
      folders: 4,
      messages: 19,
      synced_at: "2026-04-27T00:01:00Z"
    });
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
    const refreshPendingActions = vi.fn().mockResolvedValue(undefined);

    const result = await runAutomaticAccountSync({
      selectedAccountId: null,
      selectedFolderId: null,
      query: "",
      syncAccount,
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits,
      refreshPendingActions
    });

    expect(result).toEqual({ refreshed: false, status: null });
    expect(syncAccount).not.toHaveBeenCalled();
    expect(refreshFolders).not.toHaveBeenCalled();
    expect(refreshMessages).not.toHaveBeenCalled();
    expect(refreshSyncState).not.toHaveBeenCalled();
    expect(refreshAudits).not.toHaveBeenCalled();
    expect(refreshPendingActions).not.toHaveBeenCalled();
  });
});
