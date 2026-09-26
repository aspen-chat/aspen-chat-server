/**
 * Player hosts the client will put in an iframe. The server keeps the matching provider list
 * and only sends a `video` whose player is on one of these hosts; this check is the second
 * half of that allowlist, so a client never frames a host the server did not vouch for even if
 * the server's list changes first.
 */
const PLAYER_HOSTS: ReadonlySet<string> = new Set([
  "www.youtube-nocookie.com",
  "player.vimeo.com",
  "geo.dailymotion.com",
  "www.dailymotion.com",
  "streamable.com",
  "fast.wistia.net",
  "embed.ted.com",
  "www.tiktok.com",
  "player.twitch.tv",
  "clips.twitch.tv",
  "player.bilibili.com",
  "player.youku.com",
  "embed.nicovideo.jp",
  "vk.com",
  "rutube.ru",
  "ok.ru",
  "tv.naver.com",
  "www.aparat.com",
]);

/**
 * Twitch's player refuses to load unless `parent` names the embedding site's hostname, so it
 * is added here, where the hostname is known. A shell loading the app from `file://` has no
 * hostname, and there the card shows without a player.
 */
const NEEDS_PARENT: ReadonlySet<string> = new Set(["player.twitch.tv", "clips.twitch.tv"]);

/**
 * The URL to frame for a player the server sent, or `null` when it may not be framed here:
 * not `https`, not an allowlisted host, or a player that needs a hostname this page lacks.
 */
export function playerSrc(src: string, hostname: string = location.hostname): string | null {
  let url: URL;
  try {
    url = new URL(src);
  } catch {
    return null;
  }
  const host = url.hostname.toLowerCase();
  if (url.protocol !== "https:" || !PLAYER_HOSTS.has(host)) {
    return null;
  }
  if (NEEDS_PARENT.has(host)) {
    if (hostname.length === 0) {
      return null;
    }
    url.searchParams.set("parent", hostname);
  }
  return url.toString();
}
