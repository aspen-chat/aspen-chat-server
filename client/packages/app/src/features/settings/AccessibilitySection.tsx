import { ANNOUNCE_MESSAGES } from "@aspen/protocol";
import { usePreference, useSync } from "@/api/hooks";
import { planeClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";

/**
 * What helps with assistive technology: whether messages arriving in the conversation on screen
 * are read out (`ANNOUNCE_MESSAGES`), kept with this install.
 */
export function AccessibilitySection() {
  const m = useMessages();
  const sync = useSync();
  const announce = usePreference(ANNOUNCE_MESSAGES);
  return (
    <section aria-labelledby="settings-accessibility" className={planeClass}>
      <h3 id="settings-accessibility" className="text-sm font-semibold text-ink-muted">
        {m.settings.accessibility}
      </h3>
      <ChoiceCheckbox
        isSelected={announce}
        onChange={(selected) => {
          void sync.preferences.set(ANNOUNCE_MESSAGES, selected);
        }}
        label={m.settings.announceMessages}
        hint={m.settings.announceMessagesHint}
      />
    </section>
  );
}
