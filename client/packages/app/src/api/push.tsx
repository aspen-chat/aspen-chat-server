import { RelayClient, syncPush } from "@aspen/protocol";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { useSources, type Source } from "./everywhere";
import { detectShell } from "@/config";
import { channelLink, messageLink, threadLink } from "@/features/messages/links";
import { AspenPush, loadState, type NotificationTarget } from "./pushBridge";

/**
 * On the mobile app: asks to notify, registers the phone with its platform and relay, keeps a
 * subscription with every deployment the user is signed in to as they come and go, and opens a
 * tapped notification's message. Elsewhere, nothing: the desktop and web apps are told of
 * everything over their open event streams.
 */
export function WakeThisPhone() {
  const sources = useSources();
  const navigate = useNavigate();
  const token = useRef<string | null>(null);
  const syncing = useRef<Promise<void>>(Promise.resolve());
  const latest = useRef<readonly Source[]>(sources);

  const sync = useRef(() => {
    const device = token.current;
    if (device === null) {
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
        console.info("push is not available on this build", error);
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
      const tapped = await PushNotifications.addListener(
        "pushNotificationActionPerformed",
        ({ notification }) => {
          const target = notification.data as Partial<NotificationTarget> | undefined;
          const source = latest.current.find((s) => s.client.baseUrl === target?.origin);
          if (source === undefined || target?.channel == null) {
            return;
          }
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
        () => void tapped.remove(),
      );
      if (life.stopped) {
        return;
      }
      const permission = await PushNotifications.requestPermissions();
      if (permission.receive === "granted") {
        await PushNotifications.register();
      }
    })().catch((error: unknown) => {
      console.info("push is not available on this build", error);
    });
    return () => {
      life.stopped = true;
      for (const remove of removers) {
        remove();
      }
    };
  }, [navigate]);

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

  return null;
}
