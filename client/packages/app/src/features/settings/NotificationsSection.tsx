import { DESKTOP_NOTIFICATIONS, NOTIFICATION_SOUNDS } from "@aspen/protocol";
import { useState } from "react";
import { usePreference, useSync } from "@/api/hooks";
import { detectShell } from "@/config";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";
import { planeClass } from "@/features/invites/dialog";

/**
 * How this install tells the user of messages their notification settings ask for: the system's
 * notifications, which the browser must allow, and a sound. The mobile app's phone is woken by
 * push instead, so it offers only the sound.
 */
export function NotificationsSection() {
  const m = useMessages();
  const sync = useSync();
  const desktop = usePreference(DESKTOP_NOTIFICATIONS);
  const sounds = usePreference(NOTIFICATION_SOUNDS);
  const supported = typeof Notification !== "undefined";
  const [denied, setDenied] = useState(supported && Notification.permission === "denied");
  const mobile = detectShell() === "mobile";
  return (
    <section aria-labelledby="settings-notifications" className={planeClass}>
      <h3 id="settings-notifications" className="text-lg font-semibold text-ink-muted">
        {m.notifications.settings}
      </h3>
      {!mobile && (
        <ChoiceCheckbox
          isSelected={desktop && supported && !denied}
          isDisabled={!supported}
          onChange={(selected) => {
            if (!selected) {
              void sync.preferences.set(DESKTOP_NOTIFICATIONS, false);
              return;
            }
            // The browser asks the user once; its answer decides whether this can be on.
            void Notification.requestPermission().then((permission) => {
              setDenied(permission === "denied");
              void sync.preferences.set(DESKTOP_NOTIFICATIONS, permission === "granted");
            });
          }}
          label={m.notifications.desktop}
          hint={
            !supported
              ? m.notifications.desktopUnsupported
              : denied
                ? m.notifications.desktopDenied
                : m.notifications.desktopHint
          }
        />
      )}
      <ChoiceCheckbox
        isSelected={sounds}
        onChange={(selected) => {
          void sync.preferences.set(NOTIFICATION_SOUNDS, selected);
        }}
        label={m.notifications.sounds}
        hint={m.notifications.soundsHint}
      />
    </section>
  );
}
