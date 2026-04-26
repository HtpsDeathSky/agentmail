import { describe, expect, it } from "vitest";
import { api } from "./api";

describe("api demo AI bindings", () => {
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
});
