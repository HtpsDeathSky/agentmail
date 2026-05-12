import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import {
  App,
  clampWorkspaceSplitPercent,
  canStartGoogleSignIn,
  formatGoogleSignInError,
  getAccountProviderFormMode,
  getAppShellClassName,
  getMessageEnvelopeBottomEdgeMode,
  inferAccountProvider,
  getMessageEnvelopeBorderMode,
  getResponsiveMessageDetailRows,
  getWorkspaceSplitModel,
  MESSAGE_HEADER_STICKY_Z_INDEX,
  MODAL_BACKDROP_Z_INDEX,
  runGoogleSignInFlow
} from "./App";
import {
  refreshAfterMailSyncEvent,
  runDirectSendFlow,
  runInitialAccountSync,
  runManualAccountSync
} from "./lib/syncFlows";
import { getManualSyncButtonState } from "./lib/syncUi";
import { appendActivityLogEntry, buildActivityLogText } from "./lib/activityLog";
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
import {
  getContextMenuActionItems,
  shouldAutoMarkRead,
  shouldRefreshAiInsightsForAnalyzedMessage
} from "./lib/mailActions";
import type { MailMessage } from "./api";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

async function renderAppForTest() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  let root: Root | null = null;

  await act(async () => {
    root = createRoot(container);
    root.render(createElement(App));
  });

  return {
    container,
    async unmount() {
      if (!root) return;
      await act(async () => {
        root?.unmount();
      });
      container.remove();
    }
  };
}

async function findSelector(container: ParentNode, selector: string, timeoutMs = 3000) {
  const startedAt = Date.now();

  while (Date.now() - startedAt < timeoutMs) {
    const match = container.querySelector(selector);
    if (match) return match;

    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 25));
    });
  }

  throw new Error(`Selector not found: ${selector}`);
}

async function findText(container: ParentNode, text: string, timeoutMs = 3000) {
  const startedAt = Date.now();

  while (Date.now() - startedAt < timeoutMs) {
    const match = queryText(container, text);
    if (match) return match;

    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 25));
    });
  }

  throw new Error(`Text not found: ${text}`);
}

function queryText(container: ParentNode, text: string) {
  return Array.from(container.querySelectorAll("*")).find((element) => element.textContent?.trim() === text) ?? null;
}

function buildTestMessage(overrides: Partial<MailMessage> = {}): MailMessage {
  return {
    id: "msg-1",
    account_id: "acct-1",
    folder_id: "acct-1:inbox",
    uid: "42",
    message_id_header: "<msg-1@example.com>",
    subject: "Deploy report",
    sender: "ops@example.com",
    recipients: ["me@example.com"],
    cc: [],
    received_at: "2026-05-07T00:00:00.000Z",
    body_preview: "Deployment completed.",
    body: "Deployment completed.",
    html_body: null,
    inline_resources: [],
    attachments: [],
    flags: {
      is_read: false,
      is_starred: false,
      is_answered: false,
      is_forwarded: false
    },
    size_bytes: 1024,
    deleted_at: null,
    ...overrides
  };
}

describe("message detail rendering", () => {
  it("renders html message body and mail-style header fields", async () => {
    const app = await renderAppForTest();

    try {
      const htmlMessage = await findText(app.container, "HTML newsletter preview");
      await act(async () => {
        htmlMessage.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });

      const htmlBlock = await findSelector(app.container, ".body-html-block");
      const htmlFrame = htmlBlock.querySelector<HTMLIFrameElement>(".body-html-frame");
      expect(htmlFrame).not.toBeNull();
      expect(htmlFrame?.getAttribute("sandbox")).toContain("allow-popups");
      expect(htmlFrame?.getAttribute("sandbox")).toContain("allow-same-origin");
      expect(htmlFrame?.getAttribute("sandbox")).not.toContain("allow-scripts");
      expect(htmlFrame?.srcdoc).toContain("HTML Body");
      expect(htmlFrame?.srcdoc).toContain('<body class="body-mail" style="background-color: #fff; color: #111;">');
      expect(htmlFrame?.srcdoc).not.toContain('<body><div class="body-mail"');
      expect(app.container.querySelector(".metadata-grid")).toBeNull();
      expect(await findText(app.container, "Sender")).not.toBeNull();
      expect(await findText(app.container, "sender@example.com")).not.toBeNull();
      expect(await findText(app.container, "ops@example.com")).not.toBeNull();
      expect(await findText(app.container, "audit@example.com")).not.toBeNull();
    } finally {
      await app.unmount();
    }
  });
});

describe("message envelope styles", () => {
  it("uses a complete border around the mail header card with a single bottom edge", () => {
    expect(getMessageEnvelopeBorderMode()).toBe("full");
    expect(getMessageEnvelopeBottomEdgeMode()).toBe("single");
  });
});

describe("modal layer styles", () => {
  it("keeps modal backdrops above sticky message header metadata", async () => {
    const app = await renderAppForTest();

    try {
      const configButton = await findSelector(app.container, "button[title='Configuration']");

      await act(async () => {
        configButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });

      const backdrop = await findSelector(app.container, ".modal-backdrop");

      expect(Number((backdrop as HTMLElement).style.zIndex)).toBe(MODAL_BACKDROP_Z_INDEX);
      expect(MODAL_BACKDROP_Z_INDEX).toBeGreaterThan(MESSAGE_HEADER_STICKY_Z_INDEX);
    } finally {
      await app.unmount();
    }
  });
});

describe("responsive message detail layout", () => {
  it("keeps enough medium-width detail height to show the complete message header card", () => {
    const rows = getResponsiveMessageDetailRows(768, 58, 118);

    expect(rows.workspaceHeight).toBe(592);
    expect(rows.accountRailHeight).toBeCloseTo(118.4, 1);
    expect(rows.mailWorkspaceHeight).toBeCloseTo(473.6, 1);
    expect(rows.messageListHeight).toBe(180);
    expect(rows.detailPaneHeight).toBeCloseTo(293.6, 1);
    expect(rows.detailScrollHeight).toBeGreaterThan(175);
  });
});

describe("formatFolderCount", () => {
  it("shows total count when no messages are unread", () => {
    expect(formatFolderCount({ unread_count: 0, total_count: 8 })).toBe("8");
  });

  it("shows unread and total counts when unread messages exist", () => {
    expect(formatFolderCount({ unread_count: 3, total_count: 8 })).toBe("3/8");
  });
});

describe("mail action helpers", () => {
  it("auto-marks only unread messages after selection", () => {
    expect(shouldAutoMarkRead(buildTestMessage({ flags: { ...buildTestMessage().flags, is_read: false } }))).toBe(true);
    expect(shouldAutoMarkRead(buildTestMessage({ flags: { ...buildTestMessage().flags, is_read: true } }))).toBe(false);
    expect(shouldAutoMarkRead(null)).toBe(false);
  });

  it("builds context menu items for a message target without depending on selection", () => {
    expect(
      getContextMenuActionItems(buildTestMessage()).map((item) => [
        item.kind,
        item.label,
        item.kind === "action" ? item.action : undefined,
        item.disabled
      ])
    ).toEqual([
      ["action", "READ", "mark_read", false],
      ["action", "STAR", "star", false],
      ["action", "DELETE", "delete", false],
      ["analyze", "ANALYZE", undefined, false]
    ]);
  });

  it("keeps read idempotent and uses unstar for already starred context menu targets", () => {
    expect(
      getContextMenuActionItems(
        buildTestMessage({
          flags: {
            is_read: true,
            is_starred: true,
            is_answered: false,
            is_forwarded: false
          }
        })
      ).map((item) => [
        item.kind,
        item.label,
        item.kind === "action" ? item.action : undefined,
        item.disabled
      ])
    ).toEqual([
      ["action", "READ", "mark_read", true],
      ["action", "UNSTAR", "unstar", false],
      ["action", "DELETE", "delete", false],
      ["analyze", "ANALYZE", undefined, false]
    ]);
  });

  it("refreshes AI insights only when the analyzed target is the open message", () => {
    expect(shouldRefreshAiInsightsForAnalyzedMessage("msg-1", "msg-1")).toBe(true);
    expect(shouldRefreshAiInsightsForAnalyzedMessage("msg-2", "msg-1")).toBe(false);
    expect(shouldRefreshAiInsightsForAnalyzedMessage("msg-2", null)).toBe(false);
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

describe("getAccountProviderFormMode", () => {
  it("uses google sign-in controls for Gmail accounts", () => {
    expect(getAccountProviderFormMode("gmail")).toEqual({
      showPasswordField: false,
      showGoogleSignIn: true,
      testConnectionEnabled: false
    });
  });
});

describe("inferAccountProvider", () => {
  it("keeps legacy gmail-address accounts on generic IMAP/SMTP unless provider is Gmail", () => {
    expect(
      inferAccountProvider({
        id: "legacy-gmail",
        display_name: "Legacy Gmail",
        email: "legacy@gmail.com",
        provider: "generic_imap_smtp",
        imap_host: "imap.gmail.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.gmail.com",
        smtp_port: 465,
        smtp_tls: true,
        sync_enabled: true,
        created_at: "2026-05-07T00:00:00.000Z",
        updated_at: "2026-05-07T00:00:00.000Z"
      })
    ).toBe("generic_imap_smtp");
  });

  it("uses Gmail OAuth controls only for persisted Gmail provider accounts", () => {
    expect(
      inferAccountProvider({
        id: "gmail-oauth",
        display_name: "Gmail OAuth",
        email: "user@gmail.com",
        provider: "gmail",
        imap_host: "imap.gmail.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.gmail.com",
        smtp_port: 465,
        smtp_tls: true,
        sync_enabled: true,
        created_at: "2026-05-07T00:00:00.000Z",
        updated_at: "2026-05-07T00:00:00.000Z"
      })
    ).toBe("gmail");
  });
});

describe("canStartGoogleSignIn", () => {
  it("allows Google sign-in only while creating a new account", () => {
    expect(canStartGoogleSignIn(null)).toBe(true);
    expect(canStartGoogleSignIn(undefined)).toBe(true);
    expect(canStartGoogleSignIn("existing-account")).toBe(false);
  });
});

describe("runGoogleSignInFlow", () => {
  it("opens the authorization URL and waits for the backend callback without prompting", async () => {
    const account = {
      id: "acct-gmail",
      display_name: "Ops Gmail",
      email: "ops@gmail.com",
      provider: "gmail" as const,
      imap_host: "imap.gmail.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.gmail.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true,
      created_at: "2026-05-06T00:00:00.000Z",
      updated_at: "2026-05-06T00:00:00.000Z"
    };
    const startGoogleOAuth = vi.fn().mockResolvedValue({
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth?client_id=test",
      verifier_id: "verifier-1",
      redirect_uri: "http://127.0.0.1:45678/oauth/google/callback"
    });
    const waitForGoogleOAuthCallback = vi.fn().mockResolvedValue(account);
    const openAuthorizationUrl = vi.fn();
    const prompt = vi.spyOn(window, "prompt");

    const result = await runGoogleSignInFlow({
      email: "ops@gmail.com",
      displayName: "",
      startGoogleOAuth,
      waitForGoogleOAuthCallback,
      openAuthorizationUrl
    });

    expect(result).toBe(account);
    expect(startGoogleOAuth).toHaveBeenCalledWith({
      email: "ops@gmail.com",
      display_name: "Gmail"
    });
    expect(openAuthorizationUrl).toHaveBeenCalledWith("https://accounts.google.com/o/oauth2/v2/auth?client_id=test");
    expect(waitForGoogleOAuthCallback).toHaveBeenCalledWith({ verifier_id: "verifier-1" });
    expect(prompt).not.toHaveBeenCalled();
  });
});

describe("formatGoogleSignInError", () => {
  it("explains when Google sign-in has not been configured for this desktop app", () => {
    expect(
      formatGoogleSignInError(
        new Error("invalid request: AGENTMAIL_GOOGLE_OAUTH_CLIENT_ID is required")
      )
    ).toBe(
      "google sign in is not configured: set AGENTMAIL_GOOGLE_OAUTH_CLIENT_ID before launching AgentMail"
    );
  });
});

describe("activity log helpers", () => {
  it("appends timestamped session log entries without mutating previous entries", () => {
    const existing = appendActivityLogEntry([], "app startup", () => new Date("2026-04-28T09:00:00Z"));
    const next = appendActivityLogEntry(existing, "manual sync started", () => new Date("2026-04-28T09:00:05Z"));

    expect(existing).toHaveLength(1);
    expect(next).toEqual([
      { id: 1, timestamp: "2026-04-28T09:00:00.000Z", message: "app startup" },
      { id: 2, timestamp: "2026-04-28T09:00:05.000Z", message: "manual sync started" }
    ]);
  });

  it("formats session log entries as one newline-delimited text block", () => {
    const text = buildActivityLogText([
      { id: 1, timestamp: "2026-04-28T09:00:00.000Z", message: "app startup" },
      { id: 2, timestamp: "2026-04-28T09:00:05.000Z", message: "manual sync started" }
    ]);

    expect(text).toContain("app startup");
    expect(text).toContain("\n");
    expect(text).toContain("manual sync started");
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
  it("syncs a saved account, refreshes observable state, and returns a useful status", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);

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
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits
    });

    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", "previous-folder", "release");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(status).toBe("account saved and initial sync complete: ops@example.com / 4 folders / 18 messages");
  });

  it("keeps the saved account path alive when initial sync fails", async () => {
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);

    const status = await runInitialAccountSync({
      accountId: "acct-1",
      email: "ops@example.com",
      folderId: null,
      query: "",
      syncAccount: vi.fn().mockRejectedValue(new Error("IMAP login rejected")),
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits
    });

    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", null, "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
    expect(status).toBe("account saved, but initial sync failed: Error: IMAP login rejected");
  });

  it("does not sync when a saved account has sync disabled", async () => {
    const syncAccount = vi.fn().mockResolvedValue({
      account_id: "acct-1",
      folders: 4,
      messages: 18,
      synced_at: "2026-04-27T00:00:00Z"
    });
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
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits
    });

    expect(syncAccount).not.toHaveBeenCalled();
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
  it("syncs the account and refreshes visible state", async () => {
    const syncAccount = vi.fn().mockResolvedValue({
      account_id: "acct-1",
      folders: 4,
      messages: 11,
      synced_at: "2026-04-28T00:00:00Z"
    });
    const refreshFolders = vi.fn().mockResolvedValue(undefined);
    const refreshMessages = vi.fn().mockResolvedValue(undefined);
    const refreshSyncState = vi.fn().mockResolvedValue(undefined);
    const refreshAudits = vi.fn().mockResolvedValue(undefined);

    const status = await runManualAccountSync({
      accountId: "acct-1",
      folderId: "acct-1:inbox",
      query: "",
      syncAccount,
      refreshFolders,
      refreshMessages,
      refreshSyncState,
      refreshAudits
    });

    expect(status).toBe("sync complete: 4 folders / 11 messages");
    expect(syncAccount).toHaveBeenCalledWith("acct-1");
    expect(refreshFolders).toHaveBeenCalledWith("acct-1");
    expect(refreshMessages).toHaveBeenCalledWith("acct-1", "acct-1:inbox", "");
    expect(refreshSyncState).toHaveBeenCalledWith("acct-1");
    expect(refreshAudits).toHaveBeenCalled();
  });
});
