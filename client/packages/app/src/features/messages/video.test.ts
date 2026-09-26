import { describe, expect, it } from "vitest";
import { playerSrc } from "./video";

describe("playerSrc", () => {
  it("frames only https players on allowlisted hosts", () => {
    expect(playerSrc("https://www.youtube-nocookie.com/embed/x?feature=oembed", "chat.test")).toBe(
      "https://www.youtube-nocookie.com/embed/x?feature=oembed",
    );
    expect(playerSrc("https://player.bilibili.com/player.html?bvid=BV1", "chat.test")).toBe(
      "https://player.bilibili.com/player.html?bvid=BV1",
    );
    expect(playerSrc("http://www.youtube-nocookie.com/embed/x", "chat.test")).toBeNull();
    expect(playerSrc("https://www.youtube.com/embed/x", "chat.test")).toBeNull();
    expect(playerSrc("https://evil.example.org/embed/x", "chat.test")).toBeNull();
    expect(playerSrc("javascript:alert(1)", "chat.test")).toBeNull();
    expect(playerSrc("not a url", "chat.test")).toBeNull();
  });

  it("gives Twitch the embedding hostname, or no player without one", () => {
    expect(playerSrc("https://player.twitch.tv/?video=1", "chat.test")).toBe(
      "https://player.twitch.tv/?video=1&parent=chat.test",
    );
    expect(playerSrc("https://clips.twitch.tv/embed?clip=a", "chat.test")).toBe(
      "https://clips.twitch.tv/embed?clip=a&parent=chat.test",
    );
    expect(playerSrc("https://player.twitch.tv/?video=1", "")).toBeNull();
  });
});
