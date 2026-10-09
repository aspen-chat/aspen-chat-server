import {
  DESKTOP_NOTIFICATIONS,
  NOTIFICATION_SOUNDS,
  pluginText,
  type Message,
  type PluginNotice,
} from "@aspen/protocol";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useContext, useEffect, useRef } from "react";
import { useSources, type Source } from "@/api/everywhere";
import { HomeSyncContext } from "@/api/syncContext";
import { detectShell } from "@/config";
import { channelLink, messageLink, threadLink } from "@/features/messages/links";
import { notificationOutputDevice } from "@/features/settings/audioDevices";
import { useMessages } from "@/i18n/context";
import { describe } from "./describe";
import { playSound } from "./sounds";

/**
 * Tells the user of new messages their notification settings ask for (`AspenSync.onNotify`), and
 * of what plugins notify them of (`AspenSync.onPluginNotice`, which the server sends only where
 * those settings would tell of a message that tags them), on every deployment they use: a sound
 * (`NOTIFICATION_SOUNDS`), and, where they turned it on (`DESKTOP_NOTIFICATIONS`) and the browser
 * allows, the system's notification, which opens the message (or a notice's channel) when
 * clicked. Nothing for the conversation the user is looking at, nothing at all in do not
 * disturb (the home's, which holds on every deployment), and no system notification in the
 * mobile app, whose phone is woken by push instead.
 */
export function NotifyOnMessages() {
  const m = useMessages();
  const sources = useSources();
  const home = useContext(HomeSyncContext);
  const navigate = useNavigate();
  const params = useParams({ strict: false });
  const shown = useRef(params);
  useEffect(() => {
    shown.current = params;
  }, [params]);

  useEffect(() => {
    if (home === null) {
      return;
    }
    const mobile = detectShell() === "mobile";
    const tell = (
      source: Source,
      channelId: string,
      describe: () => { title: string; body: string; tag: string; open: () => void },
    ) => {
      if (home.store.doNotDisturb()) {
        return;
      }
      const looking =
        document.visibilityState === "visible" &&
        document.hasFocus() &&
        (shown.current.domain ?? null) === source.domain &&
        (shown.current.channelId === channelId || shown.current.threadId === channelId);
      if (looking) {
        return;
      }
      // The mobile app sounds only while open; closed, its phone's own notification does.
      if (
        home.preferences.get(NOTIFICATION_SOUNDS) &&
        (!mobile || document.visibilityState === "visible")
      ) {
        void playSound("chime", notificationOutputDevice(home.preferences));
      }
      if (
        mobile ||
        !home.preferences.get(DESKTOP_NOTIFICATIONS) ||
        typeof Notification === "undefined" ||
        Notification.permission !== "granted"
      ) {
        return;
      }
      const { title, body, tag, open } = describe();
      const notification = new Notification(title, { body, tag });
      notification.onclick = () => {
        window.focus();
        notification.close();
        open();
      };
    };
    const onMessage = (source: Source, message: Message) => {
      tell(source, message.channelId, () => ({
        ...describe(m, source.sync, message),
        tag: `${source.domain ?? ""}/${message.channelId}/${message.id}`,
        open: () => {
          const channel = source.sync.store.channel(message.channelId);
          const parent = channel?.parentChannel ?? null;
          const place = parent === null ? channel : source.sync.store.channel(parent);
          const where = { domain: source.domain, community: place?.community ?? null };
          void navigate(
            parent === null
              ? messageLink(where, message.channelId, message.id)
              : threadLink(where, parent, message.channelId),
          );
        },
      }));
    };
    const onNotice = (source: Source, notice: PluginNotice) => {
      const plugin = source.sync.store.plugin(notice.plugin);
      if (plugin === undefined) {
        return;
      }
      tell(source, notice.channel, () => ({
        title: plugin.name,
        body: pluginText(plugin, notice.text),
        tag: `${source.domain ?? ""}/notice/${notice.id}`,
        open: () => {
          const where = { domain: source.domain, community: notice.community ?? null };
          const parent = notice.parentChannel ?? null;
          void navigate(
            parent !== null
              ? threadLink(where, parent, notice.channel)
              : notice.message != null
                ? messageLink(where, notice.channel, notice.message)
                : channelLink(where, notice.channel),
          );
        },
      }));
    };
    const stops = sources.flatMap((source) => [
      source.sync.onNotify((message) => {
        onMessage(source, message);
      }),
      source.sync.onPluginNotice((notice) => {
        onNotice(source, notice);
      }),
    ]);
    return () => {
      for (const stop of stops) {
        stop();
      }
    };
  }, [sources, home, m, navigate]);

  return null;
}
