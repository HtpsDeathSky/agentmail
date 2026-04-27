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
});
