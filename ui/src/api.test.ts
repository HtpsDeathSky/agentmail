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

  it("shows queued sends in Sent immediately and removes the placeholder on reject", async () => {
    const account = await api.saveAccountConfig({
      id: null,
      display_name: "Queued Send Demo",
      email: "queued-send-demo@example.com",
      password: "plain-mail-secret",
      imap_host: "imap.queued-send.example.com",
      imap_port: 993,
      imap_tls: true,
      smtp_host: "smtp.queued-send.example.com",
      smtp_port: 465,
      smtp_tls: true,
      sync_enabled: true
    });

    const pendingId = await api.sendMessage({
      account_id: account.id,
      to: ["sec@example.com"],
      cc: ["ops-lead@example.com"],
      subject: "Demo queued send",
      body: "Visible before confirmation"
    });

    const pending = (await api.listPendingActions(account.id)).find((action) => action.id === pendingId);
    expect(pending).toBeDefined();
    expect(pending?.draft?.subject).toBe("Demo queued send");
    expect(pending?.draft?.message_id_header).toMatch(/^<.+@agentmail\.local>$/);
    expect(pending?.local_message_id).toBeTruthy();

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
    const placeholder = sentMessages.find((message) => message.id === pending?.local_message_id);
    expect(placeholder).toBeDefined();
    expect(placeholder?.uid).toBeNull();
    expect(placeholder?.message_id_header).toBe(pending?.draft?.message_id_header);
    expect(placeholder?.sender).toBe(account.email);
    expect(placeholder?.recipients).toEqual(["sec@example.com"]);
    expect(placeholder?.cc).toEqual(["ops-lead@example.com"]);
    expect(placeholder?.subject).toBe("Demo queued send");
    expect(placeholder?.body).toBe("Visible before confirmation");

    await api.rejectAction(pendingId);

    expect((await api.listPendingActions(account.id)).some((action) => action.id === pendingId)).toBe(false);
    await expect(
      api.listMessages({
        account_id: account.id,
        folder_id: sentFolder?.id,
        limit: 10,
        offset: 0
      })
    ).resolves.toEqual([]);
    const sentFolderAfterReject = (await api.listFolders(account.id)).find((folder) => folder.id === sentFolder?.id);
    expect(sentFolderAfterReject?.total_count).toBe(0);
    expect(sentFolderAfterReject?.unread_count).toBe(0);
  });
});
