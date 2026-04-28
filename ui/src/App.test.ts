import { describe, expect, it, vi } from "vitest";
import {
  clampWorkspaceSplitPercent,
  getAppShellClassName,
  getWorkspaceSplitModel
} from "./App";
import {
  refreshAfterMailSyncEvent,
  runDirectSendFlow,
  runInitialAccountSync,
  runManualAccountSync
} from "./lib/syncFlows";
import { getManualSyncButtonState } from "./lib/syncUi";
import {
  ACTIVITY_LOG_STORAGE_KEY,
  applyThemeModeToDocument,
  getNextThemeMode,
  readStoredActivityLogVisibility,
  readStoredWorkspaceSplitPercent,
  readStoredThemeMode,
  writeStoredActivityLogVisibility,
  WORKSPACE_SPLIT_STORAGE_KEY,
  THEME_MODE_STORAGE_KEY
} from "./lib/storage";
import { formatAuditLine, formatFolderCount, formatSendStatus } from "./lib/format";

describe("formatFolderCount", () => {
  it("shows total count when no messages are unread", () => {
    expect(formatFolderCount({ unread_count: 0, total_count: 8 })).toBe("8");
  });

  it("shows unread and total counts when unread messages exist", () => {
    expect(formatFolderCount({ unread_count: 3, total_count: 8 })).toBe("3/8");
  });
});

describe("formatSendStatus", () => {
  it("states that sends execute directly", () => {
    expect(formatSendStatus(["ops@example.com"])).toBe("sent to ops@example.com");
  });
});

describe("getManualSyncButtonState", () => {
  it("disables manual sync when no account is selected", () => {
    expect(getManualSyncButtonState(null, false)).toEqual({
      className: "icon-button sync-button",
      disabled: true,
      title: "Sync account"
    });
  });

  it("shows a pending state while manual sync is running", () => {
    expect(getManualSyncButtonState("acct-1", true)).toEqual({
      className: "icon-button sync-button syncing",
      disabled: true,
      title: "Sync running"
    });
  });
});

describe("runDirectSendFlow", () => {
  const draft = {
    account_id: "acct-1",
    to: ["ops@example.com"],
    cc: [],
    subject: "Deploy status",
    body: "Ship it"
  };

  it("reports a sent status when post-send refresh fails", async () => {
    const result = await runDirectSendFlow({
      draft,
      selectedFolderId: "acct-1:sent",
      query: "release",
      sendMessage: vi.fn().mockResolvedValue({ message_id: "sent-id", warning: null }),
      refreshFolders: vi.fn().mockRejectedValue(new Error("folder index unavailable")),
      refreshMessages: vi.fn().mockResolvedValue(undefined),
      refreshAudits: vi.fn().mockResolvedValue(undefined)
    });

    expect(result).toEqual({
      ok: true,
      status: "sent to ops@example.com / refresh failed: Error: folder index unavailable"
    });
  });

  it("keeps sent status visible when the backend reports a post-send warning", async () => {
    const result = await runDirectSendFlow({
      draft,
      selectedFolderId: "acct-1:sent",
      query: "",
      sendMessage: vi.fn().mockResolvedValue({
        message_id: "sent-id",
        warning: "sent but local persistence failed: database is locked"
      }),
      refreshFolders: vi.fn().mockResolvedValue(undefined),
      refreshMessages: vi.fn().mockResolvedValue(undefined),
      refreshAudits: vi.fn().mockResolvedValue(undefined)
    });

    expect(result).toEqual({
      ok: true,
      status: "sent to ops@example.com / sent but local persistence failed: database is locked"
    });
  });

  it("preserves the original send error when audit refresh also fails", async () => {
    const sendError = new Error("SMTP authentication rejected");

    const result = await runDirectSendFlow({
      draft,
      selectedFolderId: "acct-1:sent",
      query: "",
      sendMessage: vi.fn().mockRejectedValue(sendError),
      refreshFolders: vi.fn().mockResolvedValue(undefined),
      refreshMessages: vi.fn().mockResolvedValue(undefined),
      refreshAudits: vi.fn().mockRejectedValue(new Error("audit database locked"))
    });

    expect(result).toEqual({
      ok: false,
      status: "send failed: Error: SMTP authentication rejected",
      error: sendError
    });
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

describe("activity log visibility helpers", () => {
  it("defaults to hidden when no saved preference exists", () => {
    window.localStorage.removeItem(ACTIVITY_LOG_STORAGE_KEY);

    expect(readStoredActivityLogVisibility(window.localStorage)).toBe(false);
  });

  it("reads a saved visible preference", () => {
    window.localStorage.setItem(ACTIVITY_LOG_STORAGE_KEY, "true");

    expect(readStoredActivityLogVisibility(window.localStorage)).toBe(true);
  });

  it("persists visible and hidden preferences without throwing", () => {
    expect(() => writeStoredActivityLogVisibility(window.localStorage, true)).not.toThrow();
    expect(window.localStorage.getItem(ACTIVITY_LOG_STORAGE_KEY)).toBe("true");

    expect(() => writeStoredActivityLogVisibility(window.localStorage, false)).not.toThrow();
    expect(window.localStorage.getItem(ACTIVITY_LOG_STORAGE_KEY)).toBe("false");
  });

  it("keeps the footer class out of the default app shell", () => {
    expect(getAppShellClassName(false)).toBe("app-shell");
    expect(getAppShellClassName(true)).toBe("app-shell activity-log-visible");
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
      refreshAudits
    });

    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", "previous-folder", "release");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(startAccountWatchers).toHaveBeenCalledWith("acct-1");
    expect(status).toBe("account saved and initial sync complete: ops@example.com / 4 folders / 18 messages");
  });

  it("keeps the saved account path alive when watcher startup fails after a successful sync", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
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
      refreshAudits
    });

    expect(startAccountWatchers).toHaveBeenCalledWith("acct-1");
    expect(status).toBe("account saved and initial sync complete: ops@example.com / 4 folders / 18 messages");
  });

  it("keeps the saved account path alive when initial sync fails and skips watcher startup", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);
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
      refreshAudits
    });

    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", null, "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
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
      refreshAudits
    });

    expect(syncAccount).not.toHaveBeenCalled();
    expect(startAccountWatchers).not.toHaveBeenCalled();
    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", null, "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(status).toBe("account configuration saved: ops@example.com");
  });
});

describe("refreshAfterMailSyncEvent", () => {
  it("refreshes visible account state after a watcher sync event", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);

    const didRefresh = await refreshAfterMailSyncEvent({
      payload: { account_id: "acct-1", folder_id: "acct-1:inbox", reason: "watch_changed" },
      selectedAccountId: "acct-1",
      selectedFolderId: "acct-1:inbox",
      query: "",
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits
    });

    expect(didRefresh).toBe(true);
    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", "acct-1:inbox", "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
  });

  it("ignores watcher sync events for a different selected account", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);

    const didRefresh = await refreshAfterMailSyncEvent({
      payload: { account_id: "acct-2", folder_id: "acct-2:inbox", reason: "watch_changed" },
      selectedAccountId: "acct-1",
      selectedFolderId: "acct-1:inbox",
      query: "",
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits
    });

    expect(didRefresh).toBe(false);
    expect(refreshFolders).not.toHaveBeenCalled();
    expect(refreshMessages).not.toHaveBeenCalled();
    expect(refreshSyncState).not.toHaveBeenCalled();
    expect(refreshAudits).not.toHaveBeenCalled();
  });
});

describe("runManualAccountSync", () => {
  it("syncs the account, restarts watchers, and refreshes visible state", async () => {
    const syncAccount = vi.fn().mockResolvedValue({
      account_id: "acct-1",
      folders: 4,
      messages: 11,
      synced_at: "2026-04-28T00:00:00Z"
    });
    const startAccountWatchers = vi.fn().mockResolvedValue(undefined);
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);

    const status = await runManualAccountSync({
      accountId: "acct-1",
      folderId: "acct-1:inbox",
      query: "",
      syncAccount,
      startAccountWatchers,
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits
    });

    expect(status).toBe("sync complete: 4 folders / 11 messages");
    expect(syncAccount).toHaveBeenCalledWith("acct-1");
    expect(startAccountWatchers).toHaveBeenCalledWith("acct-1");
    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", "acct-1:inbox", "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
  });
});
