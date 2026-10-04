import { describe, expect, it } from "vitest";
import { pluginKey, pluginText } from "../src/plugins";
import type { PluginInfo } from "../src/storeTypes";

const filter: PluginInfo = {
  id: "org.example.filter",
  version: "1.0.0",
  name: "Filter",
  description: "Filters",
  mode: "optIn",
  dms: false,
  principal: null,
  principalPermissions: [],
  communitySettings: [],
  messages: { watched: "Mentions %{word}", plain: "Plain" },
};

describe("pluginText", () => {
  it("fills a plugin's placeholders from its catalogue", () => {
    expect(pluginText(filter, { key: "watched", args: { word: "pizza" } })).toBe("Mentions pizza");
  });

  it("leaves a placeholder it has no value for, and a key it has no text for", () => {
    expect(pluginText(filter, { key: "watched" })).toBe("Mentions %{word}");
    expect(pluginKey(filter, "missing")).toBe("missing");
    expect(pluginKey(filter, "plain")).toBe("Plain");
  });
});
