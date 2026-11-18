import {
  DESKTOP_NOTIFICATIONS,
  NOTIFICATION_SOUNDS,
  pluginText,
  type AspenSync,
  type Message,
  type PluginNotice,
} from "@aspen/protocol";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useContext, useEffect, useRef } from "react";
import { useAccountsHub } from "@/api/accounts";
import { useSources, type Source } from "@/api/everywhere";
import { HomeSyncContext } from "@/api/syncContext";
import { detectShell } from "@/config";
import { channelLink, messageLink, threadLink } from "@/features/messages/links";
import { notificationOutputDevice } from "@/features/settings/audioDevices";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { describe } from "./describe";
import { messageBody, ReadBudget } from "./messageBody";
import { playSound } from "./sounds";
import { tell, type Described, type Shown, type Telling } from "./tell";

/** Each deployment's budget of readings of its wording of a message. */
const budgets = new WeakMap<AspenSync, ReadBudget>();

function budgetOf(sync: AspenSync): ReadBudget {
  let budget = budgets.get(sync);
  if (budget === undefined) {
    budget = new ReadBudget();
    budgets.set(sync, budget);
  }
  return budget;
}

/**
 * Tells an account of new messages its notification settings ask for (`AspenSync.onNotify`), and
 * of what plugins notify them of (`AspenSync.onPluginNotice`, which the server sends only where
 * those settings would tell of a message that tags them), on every deployment they use: a sound
 * (`NOTIFICATION_SOUNDS`), and, where they turned it on (`DESKTOP_NOTIFICATIONS`) and the browser
 * allows, the system's notification, which opens the message (or a notice's channel) when
 * clicked. A message's notification says what its deployment says one says
 * (`AspenSync.notificationText`, as a phone's does), read once it is known the notification
 * will be shown, and the app's own wording (`describe`) where that cannot be had in time
 * (`messageBody`). Nothing for the conversation the user is looking at, nothing at all in do
 * not disturb (the home's, which holds on every deployment), and no system notification in the
 * mobile app, whose phone is woken by push instead. With several accounts held, a notification
 * names the account it is for, and opening it brings that account to the front.
 */
export function NotifyOnMessages() {
  const m = useMessages();
  const sources = useSources();
  const home = useContext(HomeSyncContext);
  const accounts = useAccountsHub();
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
    let mounted = true;
    const telling = (source: Source, channelId: string): Telling => ({
      quiet: () =>
        !mounted ||
        home.store.doNotDisturb() ||
        (document.visibilityState === "visible" &&
          document.hasFocus() &&
          accounts.isFront(source.accountId) &&
          (shown.current.domain ?? null) === source.domain &&
          (shown.current.channelId === channelId || shown.current.threadId === channelId)),
      // The mobile app sounds only while open; closed, its phone's own notification does.
      chime: () => {
        if (
          home.preferences.get(NOTIFICATION_SOUNDS) &&
          (!mobile || document.visibilityState === "visible")
        ) {
          void playSound("chime", notificationOutputDevice(home.preferences));
        }
      },
      shows: () =>
        !mobile &&
        home.preferences.get(DESKTOP_NOTIFICATIONS) &&
        typeof Notification !== "undefined" &&
        Notification.permission === "granted",
      show: ({ title, body, tag, open }: Shown) => {
        const me = home.store.me();
        const notification = new Notification(
          accounts.several && me !== null
            ? format(m.accounts.notificationTitle, { title, account: displayNameOf(me) })
            : title,
          { body, tag: `${source.accountId}/${tag}` },
        );
        notification.onclick = () => {
          window.focus();
          notification.close();
          accounts.bringToFront(source.accountId);
          open();
        };
      },
    });
    const onMessage = (source: Source, message: Message) => {
      tell(telling(source, message.channelId), (): Described => {
        const { title, body } = describe(m, source.sync, message);
        return {
          title,
          // Past the budget of readings the app's own wording is shown at once.
          body: budgetOf(source.sync).take(Date.now())
            ? messageBody(() => source.sync.notificationText(message.id), body).then((body) =>
                source.sync.store.message(message.id) === undefined ? null : body,
              )
            : body,
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
        };
      });
    };
    const onNotice = (source: Source, notice: PluginNotice) => {
      const plugin = source.sync.store.plugin(notice.plugin);
      if (plugin === undefined) {
        return;
      }
      tell(telling(source, notice.channel), () => ({
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
      mounted = false;
      for (const stop of stops) {
        stop();
      }
    };
  }, [sources, home, m, navigate, accounts]);

  return null;
}
