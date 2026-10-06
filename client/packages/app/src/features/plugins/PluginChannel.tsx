import type { Channel, PluginInfo } from "@aspen/protocol";
import {
  CalendarBlankIcon,
  ChatsCircleIcon,
  GameControllerIcon,
  ListBulletsIcon,
  NotebookIcon,
  PuzzlePieceIcon,
} from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMe, usePluginKind, useSync } from "@/api/hooks";
import { ChannelHeader } from "@/features/channels/ChannelHeader";
import { channelLink, messageLink, useDomain } from "@/features/messages/links";
import { useLanguageSetting, useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

type Kind = PluginInfo["channelTypes"][number];

/** The icon of a channel of a plugin's kind: its kind's glyph, or a puzzle piece for none. */
export function PluginGlyph({
  glyph,
  size,
  className,
}: {
  glyph: Kind["glyph"] | undefined;
  size: number;
  className?: string;
}) {
  const Icon =
    glyph === "board"
      ? NotebookIcon
      : glyph === "calendar"
        ? CalendarBlankIcon
        : glyph === "list"
          ? ListBulletsIcon
          : glyph === "chat"
            ? ChatsCircleIcon
            : glyph === "game"
              ? GameControllerIcon
              : PuzzlePieceIcon;
  return <Icon size={size} aria-hidden="true" className={className} />;
}

/**
 * A channel of a plugin's kind: its header, and the plugin's view of it, or word that it needs a
 * plugin that does not run here when no plugin the app knows declares its kind.
 */
export function PluginChannelScreen({
  channel,
  communityId,
}: {
  channel: Channel;
  communityId: string;
}) {
  const m = useMessages();
  const found = usePluginKind(channel);
  return (
    <main className="flex min-h-0 min-w-0 flex-1 flex-col">
      <ChannelHeader
        communityId={communityId}
        channelId={channel.id}
        glyph={<PluginGlyph glyph={found?.kind.glyph} size={18} />}
        name={channel.name}
      />
      {found === undefined ? (
        <p className="flex flex-1 items-center justify-center p-6 text-center text-ink-muted">
          {m.plugins.needsPlugin}
        </p>
      ) : (
        <PluginView
          key={channel.id}
          plugin={found.plugin}
          kind={found.kind}
          channel={channel}
          communityId={communityId}
        />
      )}
    </main>
  );
}

/** The app's colours by token, as a view's bridge hands them over (`spec/plugins.md`, Views). */
const THEME_TOKENS = [
  "surface",
  "surface-raised",
  "surface-sunken",
  "surface-hover",
  "ink",
  "ink-muted",
  "ink-faint",
  "line",
  "accent",
  "accent-strong",
  "accent-soft",
  "accent-contrast",
  "danger",
  "danger-soft",
  "online",
  "away",
] as const;

interface BridgeTheme {
  colors: Record<string, string>;
  fonts: { sans: string; mono: string };
  scheme: "light" | "dark";
}

/**
 * The theme as drawn now: each token's colour resolved (the palette's `light-dark()` pairs
 * settled by the mode), the font stacks, and whether it is light or dark.
 */
function readTheme(): BridgeTheme {
  const probe = document.createElement("span");
  probe.style.display = "none";
  document.body.append(probe);
  const colors: Record<string, string> = {};
  for (const token of THEME_TOKENS) {
    probe.style.color = `var(--color-${token})`;
    colors[token] = getComputedStyle(probe).color;
  }
  probe.style.color = "light-dark(rgb(0, 0, 0), rgb(255, 255, 255))";
  const scheme = getComputedStyle(probe).color === "rgb(255, 255, 255)" ? "dark" : "light";
  probe.remove();
  const root = getComputedStyle(document.documentElement);
  const emoji = root.getPropertyValue("--font-emoji");
  return {
    colors,
    fonts: {
      sans: withoutFamilies(root.getPropertyValue("--font-sans"), emoji),
      mono: withoutFamilies(root.getPropertyValue("--font-mono"), emoji),
    },
    scheme,
  };
}

/**
 * A font stack for a view, without the families of `drop`. A view cannot load the app's own
 * faces (its origin is opaque), so each family it is handed is whatever the system installs by
 * that name, and the system's Noto Color Emoji has glyphs for the digits and the space, which it
 * would draw every number and gap in. The system draws emoji in its emoji font without being
 * asked, so the emoji family is left out of a view's stacks.
 */
function withoutFamilies(stack: string, drop: string): string {
  const families = (list: string) =>
    list
      .split(",")
      .map((family) => family.trim())
      .filter((family) => family !== "");
  const dropped = new Set(families(drop));
  return families(stack)
    .filter((family) => !dropped.has(family))
    .join(", ");
}

/** The theme, read again whenever the palette, mode, fonts, or the system's preference change. */
function useBridgeTheme(): BridgeTheme {
  const [theme, setTheme] = useState(readTheme);
  useEffect(() => {
    const update = () => {
      const next = readTheme();
      setTheme((current) => (JSON.stringify(current) === JSON.stringify(next) ? current : next));
    };
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["style", "data-theme", "class"],
    });
    // Only a signal to read the theme again: with the mode following the system, the colours
    // change with no change to the document.
    const dark = window.matchMedia("(prefers-color-scheme: dark)");
    dark.addEventListener("change", update);
    return () => {
      observer.disconnect();
      dark.removeEventListener("change", update);
    };
  }, []);
  return theme;
}

/** The most people one `users` question may name. */
const MAX_USERS_ASKED = 100;

const isString = (value: unknown): value is string => typeof value === "string";

/**
 * A plugin's view of a channel: its page in a frame sandboxed as the server serves it, with no
 * shared origin and no session, and the app's side of the bridge (`spec/plugins.md`, Views).
 * The frame's only way out is asking the app, which calls the plugin's routes as the person
 * (beneath them and nowhere else), names people, passes on the plugin's events for the channel,
 * its community, and the person, and opens what the person may open.
 */
function PluginView({
  plugin,
  kind,
  channel,
  communityId,
}: {
  plugin: PluginInfo;
  kind: Kind;
  channel: Channel;
  communityId: string;
}) {
  const m = useMessages();
  const sync = useSync();
  const me = useMe();
  const domain = useDomain();
  const navigate = useNavigate();
  const { resolved } = useLanguageSetting();
  const theme = useBridgeTheme();
  const frame = useRef<HTMLIFrameElement>(null);

  // What `hello` says, kept current for the listener, which outlives renders.
  const context = useMemo(
    () => ({
      plugin: plugin.id,
      view: kind.view,
      channel: channel.id,
      channelName: channel.name,
      community: communityId,
      user: me === null ? null : { id: me.id, name: me.name, displayName: me.displayName ?? null },
      locale: resolved.locale,
      dir: resolved.direction,
      messages: plugin.messages,
      apiBase: sync.apiBase,
      theme,
    }),
    [plugin, kind.view, channel, communityId, me, resolved, sync, theme],
  );
  const latest = useRef(context);
  useEffect(() => {
    latest.current = context;
  }, [context]);

  const post = useCallback((message: object) => {
    frame.current?.contentWindow?.postMessage({ aspen: 1, ...message }, "*");
  }, []);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (event.source === null || event.source !== frame.current?.contentWindow) {
        return;
      }
      const data: unknown = event.data;
      if (data === null || typeof data !== "object" || (data as { aspen?: unknown }).aspen !== 1) {
        return;
      }
      const message = data as Record<string, unknown>;
      switch (message.type) {
        case "ready":
          post({ type: "hello", context: latest.current });
          break;
        case "request": {
          const { id, method, path, query, body } = message;
          if (!isString(method) || !isString(path)) {
            return;
          }
          sync
            .pluginRoute(plugin.id, {
              method,
              path,
              ...(isString(query) ? { query } : {}),
              ...(isString(body) ? { body } : {}),
            })
            .then(
              (answer) => {
                post({ type: "response", id, ...answer });
              },
              () => {
                // The deployment could not be reached.
                post({ type: "response", id, status: 0, contentType: null, body: "" });
              },
            );
          break;
        }
        case "users": {
          const { id, ids } = message;
          if (!Array.isArray(ids) || !ids.every(isString)) {
            return;
          }
          void sync.loadUsers(ids.slice(0, MAX_USERS_ASKED)).then((users) => {
            post({
              type: "users",
              id,
              users: users.flatMap((user) =>
                user === undefined
                  ? []
                  : [{ id: user.id, name: user.name, displayName: user.displayName ?? null }],
              ),
            });
          });
          break;
        }
        case "open": {
          const { channel: target, message: messageId } = message;
          if (!isString(target)) {
            return;
          }
          const opened = sync.store.channel(target);
          if (opened === undefined) {
            return;
          }
          const home = { domain, community: opened.community ?? null };
          void navigate(
            isString(messageId) ? messageLink(home, target, messageId) : channelLink(home, target),
          );
          break;
        }
      }
    };
    window.addEventListener("message", onMessage);
    return () => {
      window.removeEventListener("message", onMessage);
    };
  }, [sync, plugin.id, domain, navigate, post]);

  // The plugin's events for what this view shows, and for the person.
  useEffect(
    () =>
      sync.onPluginEvent((event) => {
        if (event.plugin !== plugin.id) {
          return;
        }
        const ours =
          event.channel === channel.id ||
          (event.channel == null && (event.community == null || event.community === communityId));
        if (ours) {
          post({ type: "event", kind: event.kind, payload: event.payload });
        }
      }),
    [sync, plugin.id, channel.id, communityId, post],
  );

  // A change of theme after the first, which `hello` carries.
  const [firstTheme] = useState(theme);
  useEffect(() => {
    if (theme !== firstTheme) {
      post({ type: "theme", theme });
    }
  }, [theme, firstTheme, post]);

  return (
    <iframe
      ref={frame}
      src={`${sync.apiBase}${kind.view}`}
      title={format(m.plugins.viewTitle, { channel: channel.name, plugin: plugin.name })}
      sandbox="allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox"
      referrerPolicy="no-referrer"
      onLoad={() => {
        post({ type: "hello", context: latest.current });
      }}
      className="min-h-0 w-full flex-1 border-0 bg-surface"
    />
  );
}
