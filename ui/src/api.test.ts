import { describe, expect, it } from "vitest";
import { api } from "./api";

describe("api demo AI bindings", () => {
  it("round-trips editable account configuration with plaintext password", async () => {
    const saved = await api.saveAccountConfig({
      id: null,
      display_name: "Ops Mail",
      email: "ops-config@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.config.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.config.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });

    const config = await api.getAccountConfig(saved.id);
    expect(config.password).toBe("plain-mail-secret");

    await api.saveAccountConfig({
      ...config,
      smtp_port: 587,
      password: "updated-mail-secret"
    });

    const updated = await api.getAccountConfig(saved.id);
    expect(updated.smtp_port).toBe(587);
    expect(updated.password).toBe("updated-mail-secret");
  });

  it("does not expose reconstructable short AI api keys", async () => {
    await api.clearAiSettings();

    const settings = await api.saveAiSettings({
      provider_name: "openai-compatible",
      base_url: "https://api.example.com/v1",
      model: "mail-model",
      api_key: "abcdefg",
      enabled: true
    });

    expect(settings.api_key_mask).toBe("****");
    expect(JSON.stringify(settings)).not.toContain("abcdefg");
  });

  it("returns concise Chinese demo AI summaries", async () => {
    await api.saveAiSettings({
      provider_name: "openai-compatible",
      base_url: "https://api.example.com/v1",
      model: "mail-model",
      api_key: "sk-demo-test",
      enabled: true
    });

    const messages = await api.listMessages({
      account_id: "demo-account",
      folder_id: "demo-account:inbox",
      limit: 1,
      offset: 0
    });
    const insight = await api.runAiAnalysis(messages[0].id);

    expect(insight.summary).toMatch(/[\u4e00-\u9fff]/);
    expect(insight.summary.length).toBeLessThanOrEqual(80);
    expect(insight.todos.every((todo) => /[\u4e00-\u9fff]/.test(todo))).toBe(true);
    expect(insight.reply_draft === "" || /[\u4e00-\u9fff]/.test(insight.reply_draft)).toBe(true);
  });

  it("supports starting account watchers in the browser demo", async () => {
    await expect(api.startAccountWatchers("demo-account")).resolves.toBeNull();
  });

  it("sends directly in the browser demo and records Sent mail", async () => {
    const account = await api.saveAccountConfig({
      id: null,
      display_name: "Direct Send Demo",
      email: "direct-send-demo@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.direct-send.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.direct-send.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });

    const sendResult = await api.sendMessage({
      account_id: account.id,
      to: ["sec@example.com"],
      cc: ["ops-lead@example.com"],
      subject: "Demo direct send",
      body: "Visible after SMTP success"
    });
    const sentMessageId = sendResult.message_id;
    expect(sendResult.warning).toBeNull();

    const sentFolder = (await api.listFolders(account.id)).find(
      (folder) => folder.account_id === account.id && folder.role === "sent"
    );
    expect(sentFolder).toBeDefined();
    expect(sentFolder?.total_count).toBe(1);
    expect(sentFolder?.unread_count).toBe(0);

    const sentMessages = await api.listMessages({
      account_id: account.id,
      folder_id: sentFolder?.id,
      limit: 10,
      offset: 0
    });
    const sentMessage = sentMessages.find((message) => message.id === sentMessageId);
    expect(sentMessage).toBeDefined();
    expect(sentMessage?.uid).toBeNull();
    expect(sentMessage?.message_id_header).toMatch(/^<.+@agentmail\.local>$/);
    expect(sentMessage?.sender).toBe(account.email);
    expect(sentMessage?.recipients).toEqual(["sec@example.com"]);
    expect(sentMessage?.cc).toEqual(["ops-lead@example.com"]);
    expect(sentMessage?.subject).toBe("Demo direct send");
    expect(sentMessage?.body).toBe("Visible after SMTP success");

    const audits = await api.getAuditLog(5);
    expect(audits[0].action).toBe("send");
    expect(audits[0].status).toBe("executed");
    expect(audits[0].message_ids).toEqual([sentMessageId]);
  });

  it("rejects direct sends with an empty recipient list before Sent mutation or audit in the browser demo", async () => {
    const account = await api.saveAccountConfig({
      id: null,
      display_name: "Empty Recipient Demo",
      email: "empty-recipient-demo@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.empty-recipient.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.empty-recipient.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.sendMessage({
        account_id: account.id,
        to: [],
        cc: [],
        subject: "No recipients",
        body: "This should not create Sent mail."
      })
    ).rejects.toThrow("recipient list is empty");

    const sentFolder = (await api.listFolders(account.id)).find((folder) => folder.account_id === account.id && folder.role === "sent");
    expect(sentFolder).toBeUndefined();
    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects direct sends with only blank recipients before Sent mutation or audit in the browser demo", async () => {
    const account = await api.saveAccountConfig({
      id: null,
      display_name: "Blank Recipient Demo",
      email: "blank-recipient-demo@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.blank-recipient.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.blank-recipient.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.sendMessage({
        account_id: account.id,
        to: ["  ", ""],
        cc: ["   "],
        subject: "Blank recipients",
        body: "This should not create Sent mail."
      })
    ).rejects.toThrow("recipient list is empty");

    const sentFolder = (await api.listFolders(account.id)).find((folder) => folder.account_id === account.id && folder.role === "sent");
    expect(sentFolder).toBeUndefined();
    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("records multi-message move actions as batch_move in the browser demo", async () => {
    const folders = await api.listFolders("demo-account");
    const archiveFolder = folders.find((folder) => folder.account_id === "demo-account" && folder.role === "archive");
    expect(archiveFolder).toBeDefined();

    const messageIds = ["msg-001", "msg-002"];
    await expect(
      api.executeMailAction({
        action: "move",
        account_id: "demo-account",
        message_ids: messageIds,
        target_folder_id: archiveFolder?.id
      })
    ).resolves.toEqual({ kind: "executed", pending_action_id: null });

    const archivedMessages = await api.listMessages({
      account_id: "demo-account",
      folder_id: archiveFolder?.id,
      limit: 20,
      offset: 0
    });
    expect(archivedMessages.filter((message) => messageIds.includes(message.id)).map((message) => message.id).sort()).toEqual(messageIds);

    const audits = await api.getAuditLog(5);
    expect(audits[0].action).toBe("batch_move");
    expect(audits[0].status).toBe("executed");
    expect(audits[0].message_ids).toEqual(messageIds);
  });

  it("executes explicit permanent_delete on Trash messages in the browser demo", async () => {
    const trashFolder = (await api.listFolders("demo-account")).find(
      (folder) => folder.account_id === "demo-account" && folder.role === "trash"
    );
    expect(trashFolder).toBeDefined();

    await expect(
      api.executeMailAction({
        action: "permanent_delete",
        account_id: "demo-account",
        message_ids: ["msg-401"]
      })
    ).resolves.toEqual({ kind: "executed", pending_action_id: null });

    const trashMessages = await api.listMessages({
      account_id: "demo-account",
      folder_id: trashFolder?.id,
      limit: 20,
      offset: 0
    });
    expect(trashMessages.some((message) => message.id === "msg-401")).toBe(false);

    const audits = await api.getAuditLog(5);
    expect(audits[0].action).toBe("permanent_delete");
    expect(audits[0].status).toBe("executed");
    expect(audits[0].message_ids).toEqual(["msg-401"]);
  });

  it("rejects send actions through execute_mail_action without auditing in the browser demo", async () => {
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.executeMailAction({
        action: "send",
        account_id: "demo-account",
        message_ids: ["msg-003"]
      })
    ).rejects.toThrow("send and forward actions must use dedicated commands");

    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects forward actions through execute_mail_action without auditing in the browser demo", async () => {
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.executeMailAction({
        action: "forward",
        account_id: "demo-account",
        message_ids: ["msg-003"]
      })
    ).rejects.toThrow("send and forward actions must use dedicated commands");

    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects move actions without target folders before mutation or audit in the browser demo", async () => {
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.executeMailAction({
        action: "move",
        account_id: "demo-account",
        message_ids: ["msg-003"]
      })
    ).rejects.toThrow("target folder is required");

    const inboxMessages = await api.listMessages({
      account_id: "demo-account",
      folder_id: "demo-account:inbox",
      limit: 20,
      offset: 0
    });
    expect(inboxMessages.some((message) => message.id === "msg-003")).toBe(true);
    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects move actions to nonexistent target folders before mutation or audit in the browser demo", async () => {
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.executeMailAction({
        action: "move",
        account_id: "demo-account",
        message_ids: ["msg-003"],
        target_folder_id: "demo-account:missing-target"
      })
    ).rejects.toThrow("target folder not found");

    const inboxMessages = await api.listMessages({
      account_id: "demo-account",
      folder_id: "demo-account:inbox",
      limit: 20,
      offset: 0
    });
    expect(inboxMessages.some((message) => message.id === "msg-003")).toBe(true);
    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects move actions to another account target folder before mutation or audit in the browser demo", async () => {
    const account = await api.saveAccountConfig({
      id: null,
      display_name: "Target Owner Demo",
      email: "target-owner-demo@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.target-owner.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.target-owner.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });
    await api.sendMessage({
      account_id: account.id,
      to: ["sec@example.com"],
      cc: [],
      subject: "Create target folder",
      body: "This direct send creates a Sent folder for the saved account."
    });
    const otherAccountSentFolder = (await api.listFolders(account.id)).find(
      (folder) => folder.account_id === account.id && folder.role === "sent"
    );
    expect(otherAccountSentFolder).toBeDefined();
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.executeMailAction({
        action: "move",
        account_id: "demo-account",
        message_ids: ["msg-003"],
        target_folder_id: otherAccountSentFolder?.id
      })
    ).rejects.toThrow("target folder does not belong to account");

    const inboxMessages = await api.listMessages({
      account_id: "demo-account",
      folder_id: "demo-account:inbox",
      limit: 20,
      offset: 0
    });
    expect(inboxMessages.some((message) => message.id === "msg-003")).toBe(true);
    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects delete when the account has no Trash folder before mutation or audit in the browser demo", async () => {
    const account = await api.saveAccountConfig({
      id: null,
      display_name: "No Trash Demo",
      email: "no-trash-demo@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.no-trash.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.no-trash.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });
    const sendResult = await api.sendMessage({
      account_id: account.id,
      to: ["sec@example.com"],
      cc: [],
      subject: "Delete requires Trash",
      body: "This message should remain in Sent when Trash is missing."
    });
    const messageId = sendResult.message_id;
    const sentFolder = (await api.listFolders(account.id)).find((folder) => folder.account_id === account.id && folder.role === "sent");
    expect(sentFolder).toBeDefined();
    expect((await api.listFolders(account.id)).some((folder) => folder.account_id === account.id && folder.role === "trash")).toBe(false);
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.executeMailAction({
        action: "delete",
        account_id: account.id,
        message_ids: [messageId]
      })
    ).rejects.toThrow("trash folder not found");

    const sentMessages = await api.listMessages({
      account_id: account.id,
      folder_id: sentFolder?.id,
      limit: 10,
      offset: 0
    });
    expect(sentMessages.some((message) => message.id === messageId)).toBe(true);
    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects archive when the account has no Archive folder before mutation or audit in the browser demo", async () => {
    const account = await api.saveAccountConfig({
      id: null,
      display_name: "No Archive Demo",
      email: "no-archive-demo@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.no-archive.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.no-archive.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });
    const sendResult = await api.sendMessage({
      account_id: account.id,
      to: ["sec@example.com"],
      cc: [],
      subject: "Archive requires Archive",
      body: "This message should remain in Sent when Archive is missing."
    });
    const messageId = sendResult.message_id;
    const sentFolder = (await api.listFolders(account.id)).find((folder) => folder.account_id === account.id && folder.role === "sent");
    expect(sentFolder).toBeDefined();
    expect((await api.listFolders(account.id)).some((folder) => folder.account_id === account.id && folder.role === "archive")).toBe(false);
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.executeMailAction({
        action: "archive",
        account_id: account.id,
        message_ids: [messageId]
      })
    ).rejects.toThrow("archive folder not found");

    const sentMessages = await api.listMessages({
      account_id: account.id,
      folder_id: sentFolder?.id,
      limit: 10,
      offset: 0
    });
    expect(sentMessages.some((message) => message.id === messageId)).toBe(true);
    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects mixed-folder batch actions before mutating or auditing in the browser demo", async () => {
    const archiveFolder = (await api.listFolders("demo-account")).find(
      (folder) => folder.account_id === "demo-account" && folder.role === "archive"
    );
    expect(archiveFolder).toBeDefined();
    const latestAuditBefore = (await api.getAuditLog(1))[0];

    await expect(
      api.executeMailAction({
        action: "batch_delete",
        account_id: "demo-account",
        message_ids: ["msg-003", "msg-201"]
      })
    ).rejects.toThrow("all messages in one action must be in the same folder");

    const inboxMessages = await api.listMessages({
      account_id: "demo-account",
      folder_id: "demo-account:inbox",
      limit: 20,
      offset: 0
    });
    const archiveMessages = await api.listMessages({
      account_id: "demo-account",
      folder_id: archiveFolder?.id,
      limit: 20,
      offset: 0
    });
    expect(inboxMessages.some((message) => message.id === "msg-003")).toBe(true);
    expect(archiveMessages.some((message) => message.id === "msg-201")).toBe(true);
    expect((await api.getAuditLog(1))[0].id).toBe(latestAuditBefore.id);
  });

  it("rejects missing and wrong-account action message ids in the browser demo", async () => {
    const account = await api.saveAccountConfig({
      id: null,
      display_name: "Validation Demo",
      email: "validation-demo@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.validation.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.validation.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });
    const otherAccountSendResult = await api.sendMessage({
      account_id: account.id,
      to: ["sec@example.com"],
      cc: [],
      subject: "Wrong account validation",
      body: "This message belongs to a different demo account."
    });
    const otherAccountMessageId = otherAccountSendResult.message_id;

    await expect(
      api.executeMailAction({
        action: "mark_read",
        account_id: "demo-account",
        message_ids: ["missing-demo-message"]
      })
    ).rejects.toThrow("message not found");

    await expect(
      api.executeMailAction({
        action: "mark_read",
        account_id: "demo-account",
        message_ids: [otherAccountMessageId]
      })
    ).rejects.toThrow("message does not belong to account");
  });

  it("executes explicit batch_delete by moving non-Trash messages to Trash in the browser demo", async () => {
    const trashFolder = (await api.listFolders("demo-account")).find(
      (folder) => folder.account_id === "demo-account" && folder.role === "trash"
    );
    expect(trashFolder).toBeDefined();

    const messageIds = ["msg-001", "msg-002"];
    await expect(
      api.executeMailAction({
        action: "batch_delete",
        account_id: "demo-account",
        message_ids: messageIds
      })
    ).resolves.toEqual({ kind: "executed", pending_action_id: null });

    const trashMessages = await api.listMessages({
      account_id: "demo-account",
      folder_id: trashFolder?.id,
      limit: 20,
      offset: 0
    });
    const movedMessages = trashMessages.filter((message) => messageIds.includes(message.id));
    expect(movedMessages.map((message) => message.id).sort()).toEqual(messageIds);
    expect(movedMessages.every((message) => message.uid === null)).toBe(true);

    const refreshedTrashFolder = (await api.listFolders("demo-account")).find((folder) => folder.id === trashFolder?.id);
    expect(refreshedTrashFolder?.total_count).toBeGreaterThanOrEqual(messageIds.length);

    const audits = await api.getAuditLog(5);
    expect(audits[0].action).toBe("batch_delete");
    expect(audits[0].status).toBe("executed");
    expect(audits[0].message_ids).toEqual(messageIds);
  });
});
