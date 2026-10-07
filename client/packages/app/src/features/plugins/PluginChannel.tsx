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
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Button } from "react-aria-components";
import { useMe, usePluginKind, useSync } from "@/api/hooks";
import { ChannelHeader } from "@/features/channels/ChannelHeader";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { channelLink, messageLink, useDomain } from "@/features/messages/links";
import { useLanguageSetting, useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { facesUnder, type FaceFile } from "@/theme/fontLibrary";
import { chosenAliases } from "@/theme/fonts";
import { VIEW_FONTS } from "../../../viewFonts";

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
  fonts: {
    sans: string;
    mono: string;
    /** Every face the app bundles, served by the view's deployment (`viewFonts.ts`). */
    stylesheet: string;
    /** The user's own faces drawn now, as files (`fontLibrary.ts`). */
    faces: FaceFile[];
  };
  scheme: "light" | "dark";
}

/** What `readTheme` reads from the document; the rest of the theme follows from it. */
interface DrawnTheme {
  colors: Record<string, string>;
  fonts: { sans: string; mono: string };
  scheme: "light" | "dark";
  /** The aliases of the user's families in the stacks. */
  chosen: string[];
}

/**
 * The theme as drawn now: each token's colour resolved (the palette's `light-dark()` pairs
 * settled by the mode), the font stacks, and whether it is light or dark.
 */
function readTheme(): DrawnTheme {
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
  return {
    colors,
    fonts: {
      sans: root.getPropertyValue("--font-sans").trim(),
      mono: root.getPropertyValue("--font-mono").trim(),
    },
    scheme,
    chosen: chosenAliases(),
  };
}

/**
 * The theme a view of `apiBase`'s deployment is handed, read again whenever the palette, mode,
 * fonts, or the system's preference change. A view cannot load the app's faces itself, so it is
 * handed the address of the stylesheet naming them on its own deployment, which serves the web
 * client's files beside its views, and the files of the user's own faces drawn now.
 */
function useBridgeTheme(apiBase: string): BridgeTheme {
  const [drawn, setDrawn] = useState(readTheme);
  useEffect(() => {
    const update = () => {
      const next = readTheme();
      setDrawn((current) => (JSON.stringify(current) === JSON.stringify(next) ? current : next));
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
  const [faces, setFaces] = useState<FaceFile[]>([]);
  const chosen = drawn.chosen.join(",");
  useEffect(() => {
    let current = true;
    facesUnder(chosen === "" ? [] : chosen.split(",")).then(
      (files) => {
        if (current) setFaces(files);
      },
      (error: unknown) => {
        // The view draws in the stacks' next faces.
        console.warn("the font library could not be read for a plugin's view", error);
      },
    );
    return () => {
      current = false;
    };
  }, [chosen]);
  return useMemo(
    () => ({
      colors: drawn.colors,
      fonts: {
        ...drawn.fonts,
        stylesheet: new URL(`/${VIEW_FONTS}`, apiBase).href,
        faces,
      },
      scheme: drawn.scheme,
    }),
    [drawn, faces, apiBase],
  );
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
 *
 * The bridge speaks only to the plugin's own page, loaded by the app. When the frame the app
 * made first loads, the app says `hello` to it, handing over one end of a `MessageChannel`, and
 * speaks over that port alone from then on, in both directions; a page the frame goes to after
 * (a link, a redirect) never holds it, since the port belongs to the page it was handed to, and
 * nothing heard on the frame's window is answered. A later load of the same frame closes the
 * port and offers to load the plugin's page again, in a new frame, which gets a new port.
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
  const theme = useBridgeTheme(sync.apiBase);
  const frame = useRef<HTMLIFrameElement>(null);
  const src = `${sync.apiBase}${kind.view}`;
  // Each frame the app makes, by its key: a new one for another page or a reload.
  const [generation, setGeneration] = useState(0);
  const frameKey = `${String(generation)} ${src}`;
  // The frame whose first load has been seen, and the app's end of the port handed to it.
  const loaded = useRef<string | null>(null);
  const port = useRef<MessagePort | null>(null);
  const [left, setLeft] = useState(false);
  const closePort = useCallback(() => {
    port.current?.close();
    port.current = null;
  }, []);
  useLayoutEffect(() => closePort, [frameKey, closePort]);

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

  /** Says `message` over the current port, if there is one. */
  const post = useCallback((message: object) => {
    port.current?.postMessage({ aspen: 1, ...message });
  }, []);

  // What the page asks over its port, kept current for the port's listener. An answer goes back
  // over the port it was asked on, and only while that port is still the frame's.
  const handle = useRef<(data: unknown, from: MessagePort) => void>(() => undefined);
  useEffect(() => {
    handle.current = (data, from) => {
      if (data === null || typeof data !== "object" || (data as { aspen?: unknown }).aspen !== 1) {
        return;
      }
      const reply = (answer: object) => {
        if (port.current === from) {
          from.postMessage({ aspen: 1, ...answer });
        }
      };
      const message = data as Record<string, unknown>;
      switch (message.type) {
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
                reply({ type: "response", id, ...answer });
              },
              () => {
                // The deployment could not be reached.
                reply({ type: "response", id, status: 0, contentType: null, body: "" });
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
            reply({
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
  }, [sync, plugin.id, domain, navigate]);

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

  if (left) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
        <p role="alert" className="max-w-md text-ink-muted">
          {format(m.plugins.viewLeft, { plugin: plugin.name })}
        </p>
        <Button
          onPress={() => {
            setLeft(false);
            setGeneration((n) => n + 1);
          }}
          className={secondaryButtonClass}
        >
          {m.plugins.reloadView}
        </Button>
      </div>
    );
  }
  return (
    <iframe
      key={frameKey}
      ref={frame}
      src={src}
      title={format(m.plugins.viewTitle, { channel: channel.name, plugin: plugin.name })}
      sandbox="allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox"
      referrerPolicy="no-referrer"
      onLoad={() => {
        closePort();
        if (loaded.current === frameKey) {
          setLeft(true);
          return;
        }
        loaded.current = frameKey;
        const pipe = new MessageChannel();
        const ours = pipe.port1;
        ours.onmessage = (event) => {
          handle.current(event.data, ours);
        };
        port.current = ours;
        frame.current?.contentWindow?.postMessage(
          { aspen: 1, type: "hello", context: latest.current },
          "*",
          [pipe.port2],
        );
      }}
      className="min-h-0 w-full flex-1 border-0 bg-surface"
    />
  );
}
