import { DESKTOP_NOTIFICATIONS, NOTIFICATION_SOUNDS, type Message } from "@aspen/protocol";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useContext, useEffect, useRef } from "react";
import { useSources, type Source } from "@/api/everywhere";
import { HomeSyncContext } from "@/api/syncContext";
import { detectShell } from "@/config";
import { messageLink, threadLink } from "@/features/messages/links";
import { notificationOutputDevice } from "@/features/settings/audioDevices";
import { useMessages } from "@/i18n/context";
import { playChime } from "./chime";
import { describe } from "./describe";

/**
 * Tells the user of new messages their notification settings ask for (`AspenSync.onNotify`), on
 * every deployment they use: a sound (`NOTIFICATION_SOUNDS`), and, where they turned it on
 * (`DESKTOP_NOTIFICATIONS`) and the browser allows, the system's notification, which opens the
 * message when clicked. Nothing for the conversation the user is looking at, and no system
 * notification in the mobile app, whose phone is woken by push instead.
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
    const tell = (source: Source, message: Message) => {
      const looking =
        document.visibilityState === "visible" &&
        document.hasFocus() &&
        (shown.current.domain ?? null) === source.domain &&
        (shown.current.channelId === message.channelId ||
          shown.current.threadId === message.channelId);
      if (looking) {
        return;
      }
      // The mobile app sounds only while open; closed, its phone's own notification does.
      if (
        home.preferences.get(NOTIFICATION_SOUNDS) &&
        (!mobile || document.visibilityState === "visible")
      ) {
        void playChime(notificationOutputDevice(home.preferences));
      }
      if (
        mobile ||
        !home.preferences.get(DESKTOP_NOTIFICATIONS) ||
        typeof Notification === "undefined" ||
        Notification.permission !== "granted"
      ) {
        return;
      }
      const { title, body } = describe(m, source.sync, message);
      const notification = new Notification(title, {
        body,
        tag: `${source.domain ?? ""}/${message.channelId}/${message.id}`,
      });
      notification.onclick = () => {
        window.focus();
        notification.close();
        const channel = source.sync.store.channel(message.channelId);
        const parent = channel?.parentChannel ?? null;
        const place = parent === null ? channel : source.sync.store.channel(parent);
        const where = { domain: source.domain, community: place?.community ?? null };
        void navigate(
          parent === null
            ? messageLink(where, message.channelId, message.id)
            : threadLink(where, parent, message.channelId),
        );
      };
    };
    const stops = sources.map((source) =>
      source.sync.onNotify((message) => {
        tell(source, message);
      }),
    );
    return () => {
      for (const stop of stops) {
        stop();
      }
    };
  }, [sources, home, m, navigate]);

  return null;
}
