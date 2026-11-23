import { RelayClient, stopPush, syncPush } from "@aspen/protocol";
import type { PushNotificationsPlugin } from "@capacitor/push-notifications";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { useAccountsHub, useAllSources } from "./accounts";
import type { Source } from "./everywhere";
import { detectShell } from "@/config";
import { channelLink, messageLink, threadLink } from "@/features/messages/links";
import { PushPrompt } from "@/features/notifications/PushPrompt";
import { AspenPush, loadState, type NotificationTarget } from "./pushBridge";
import {
  choosePush,
  notePushStatus,
  pushStatus,
  usePushStatus,
  type PushChoice,
} from "./pushChoice";

/**
 * On the mobile app: asks whether to be woken, and only on Accept asks the system to notify;
 * registers the phone with its platform and relay, keeps a subscription with every deployment the
 * person's accounts are signed in to as they come and go, and opens a tapped notification's
 * message under the account it was for. Declined,
 * or turned off in Settings, the phone is registered nowhere until it is turned on there.
 * Elsewhere, nothing: the desktop and web apps are told of everything over their open event
 * streams.
 */
export function WakeThisPhone() {
  const sources = useAllSources();
  const accounts = useAccountsHub();
  const navigate = useNavigate();
  const token = useRef<string | null>(null);
  const syncing = useRef<Promise<void>>(Promise.resolve());
  const latest = useRef<readonly Source[]>(sources);
  const plugin = useRef<PushNotificationsPlugin | null>(null);
  const [pushable, setPushable] = useState(false);
  const { choice, asking, answers } = usePushStatus();

  const sync = useRef(() => {
    const device = token.current;
    if (device === null || pushStatus().choice !== "on") {
      return;
    }
    // One at a time, each from the state the last one left.
    syncing.current = syncing.current.then(async () => {
      try {
        const facts = await AspenPush.describe();
        const state = await syncPush(
          new RelayClient(facts.relay),
          await loadState(),
          {
            platform: facts.platform,
            app: facts.app,
            environment: facts.environment,
            token: device,
          },
          latest.current.map((source) => ({
            origin: source.client.baseUrl,
            client: source.client,
          })),
        );
        await AspenPush.saveState({ state: JSON.stringify(state) });
      } catch (error) {
        console.error("push: could not register this phone with its relay and deployments", error);
      }
    });
  });

  useEffect(() => {
    if (detectShell() !== "mobile") {
      return;
    }
    const life = { stopped: false };
    const removers: (() => void)[] = [];
    void (async () => {
      const { PushNotifications } = await import("@capacitor/push-notifications");
      const registered = await PushNotifications.addListener("registration", ({ value }) => {
        token.current = value;
        sync.current();
      });
      const failed = await PushNotifications.addListener("registrationError", ({ error }) => {
        console.error("push: the platform refused to register this phone", error);
      });
      const tapped = await PushNotifications.addListener(
        "pushNotificationActionPerformed",
        ({ notification }) => {
          const target = notification.data as Partial<NotificationTarget> | undefined;
          const here = latest.current.filter((s) => s.client.baseUrl === target?.origin);
          const source =
            here.find((s) => s.client.session?.userId === target?.userId) ??
            here.find((s) => accounts.isFront(s.accountId)) ??
            here[0];
          if (source === undefined || target?.channel == null) {
            return;
          }
          accounts.bringToFront(source.accountId);
          const home = { domain: source.domain, community: target.community ?? null };
          void navigate(
            target.parentChannel != null
              ? threadLink(home, target.parentChannel, target.channel)
              : target.message != null
                ? messageLink(home, target.channel, target.message)
                : channelLink(home, target.channel),
          );
        },
      );
      removers.push(
        () => void registered.remove(),
        () => void failed.remove(),
        () => void tapped.remove(),
      );
      // A build that names no relay cannot be woken, and asks nothing of the system: Android's
      // registration asks Firebase, which such a build is not set up for, and throws on a thread
      // of its own, taking the app down. `describe` refuses on such a build, ending this here.
      try {
        await AspenPush.describe();
      } catch (error) {
        console.warn("push: this build cannot be woken", error);
        return;
      }
      if (life.stopped) {
        return;
      }
      plugin.current = PushNotifications;
      notePushStatus({ available: true });
      setPushable(true);
    })().catch((error: unknown) => {
      console.error("push: could not start", error);
    });
    return () => {
      life.stopped = true;
      for (const remove of removers) {
        remove();
      }
    };
  }, [navigate, accounts]);

  // The person's answer, as it stands and whenever Settings changes it.
  useEffect(() => {
    const notifications = plugin.current;
    if (!pushable || notifications === null) {
      return;
    }
    const follow = async (wanted: PushChoice) => {
      if (wanted === "off") {
        token.current = null;
        syncing.current = syncing.current.then(() => stopWaking(notifications, latest.current));
        return;
      }
      const { receive } = await notifications.checkPermissions();
      if (wanted === null) {
        if (receive === "granted") {
          // Allowed before this install asked: the person's answer is already the system's.
          choosePush("on");
        } else {
          if (receive === "denied") {
            console.warn("push: the system refuses notifications, so the person is not asked");
          }
          // The system refusing is its own answer, which only its settings change.
          notePushStatus({ denied: receive === "denied", asking: receive !== "denied" });
        }
        return;
      }
      const answered =
        receive === "granted" || receive === "denied"
          ? receive
          : (await notifications.requestPermissions()).receive;
      notePushStatus({ denied: answered === "denied" });
      if (answered === "granted") {
        await notifications.register();
      } else {
        console.warn(`push: the system did not allow notifications (${answered})`);
      }
    };
    const followed = () => {
      follow(choice).catch((error: unknown) => {
        console.error("push: could not follow the person's answer", error);
      });
    };
    followed();
    // The system's settings may have allowed notifications meanwhile.
    const onVisible = () => {
      if (document.visibilityState === "visible" && choice === "on" && pushStatus().denied) {
        followed();
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [pushable, choice, answers]);

  // Every deployment signed in to or out of, and every new session, is kept in step.
  useEffect(() => {
    latest.current = sources;
    sync.current();
    const stops = sources.map((source) =>
      source.client.subscribe(() => {
        sync.current();
      }),
    );
    return () => {
      for (const stop of stops) {
        stop();
      }
    };
  }, [sources]);

  return asking ? <PushPrompt /> : null;
}

/**
 * Stops waking this phone: the platform forgets its token, and each deployment and the relay
 * their subscriptions (`stopPush`). A failure keeps what was registered, which the next start
 * ends again while the person's answer is still off.
 */
async function stopWaking(
  notifications: PushNotificationsPlugin,
  sources: readonly Source[],
): Promise<void> {
  try {
    const state = await loadState();
    if (state.device === null && state.accounts.length === 0) {
      return;
    }
    await notifications.unregister();
    const facts = await AspenPush.describe();
    const stopped = await stopPush(
      new RelayClient(facts.relay),
      state,
      sources.map((source) => ({ origin: source.client.baseUrl, client: source.client })),
    );
    await AspenPush.saveState({ state: JSON.stringify(stopped) });
  } catch (error) {
    console.error("push: could not stop waking this phone", error);
  }
}
