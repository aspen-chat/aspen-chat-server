import { TYPING_NOTICES } from "@aspen/protocol";
import { usePreference, useSync } from "@/api/hooks";
import { planeClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";

/** What others are told of the user as they use Aspen: whether they see them typing. */
export function PrivacySection() {
  const m = useMessages();
  const sync = useSync();
  const typing = usePreference(TYPING_NOTICES);
  return (
    <section aria-labelledby="settings-privacy" className={planeClass}>
      <h3 id="settings-privacy" className="text-sm font-semibold text-ink-muted">
        {m.settings.privacy}
      </h3>
      <ChoiceCheckbox
        isSelected={typing}
        onChange={(selected) => {
          void sync.preferences.set(TYPING_NOTICES, selected);
        }}
        label={m.settings.typingNotices}
        hint={m.settings.typingNoticesHint}
      />
    </section>
  );
}
