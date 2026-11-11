import type { ChannelType } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { channelGlyphKind } from "./channelGlyphKind";

describe("channelGlyphKind", () => {
  it("gives a community's channel types each their mark", () => {
    expect(channelGlyphKind("text")).toBe("text");
    expect(channelGlyphKind("voice")).toBe("voice");
    expect(channelGlyphKind("plugin")).toBe("plugin");
  });

  it("gives a type this build does not know the neutral mark, not a voice channel's", () => {
    expect(channelGlyphKind("stage" as ChannelType)).toBe("other");
  });
});
