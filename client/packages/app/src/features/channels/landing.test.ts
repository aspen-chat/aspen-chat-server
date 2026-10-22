import type { Channel, ChannelType, Community } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { landingChannel } from "./landing";

const channel = (id: string, ty: ChannelType): Channel => ({
  id,
  community: "community",
  name: id,
  ty,
  sortIndex: 0,
  recipients: [],
  replyCount: 0,
});

const community = (defaultChannel: string | null): Community => ({
  id: "community",
  name: "Community",
  defaultChannel,
});

describe("landingChannel", () => {
  const channels = [channel("hall", "voice"), channel("general", "text"), channel("rules", "text")];

  it("opens the default channel", () => {
    expect(landingChannel(community("rules"), channels)?.id).toBe("rules");
  });

  it("opens the first text channel when the default is one the reader does not have", () => {
    expect(landingChannel(community("hidden"), channels)?.id).toBe("general");
  });

  it("opens the first text channel without a default, or before the community is known", () => {
    expect(landingChannel(community(null), channels)?.id).toBe("general");
    expect(landingChannel(undefined, channels)?.id).toBe("general");
  });

  it("opens nothing without a text channel", () => {
    expect(landingChannel(community(null), [channel("hall", "voice")])).toBeUndefined();
  });
});
