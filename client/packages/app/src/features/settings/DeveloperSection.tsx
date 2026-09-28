import { DEVELOPER_MODE } from "@aspen/protocol";
import { usePreference, useSync } from "@/api/hooks";
import { BotsDialog } from "@/features/bots/BotsDialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";

/** The opt-in to developer mode, and, once in it, the user's bots. */
export function DeveloperSection() {
  const m = useMessages();
  const sync = useSync();
  const on = usePreference(DEVELOPER_MODE);
  return (
    <section aria-labelledby="settings-developer" className="flex flex-col gap-3">
      <h3 id="settings-developer" className="text-sm font-semibold text-ink-muted">
        {m.bots.developerMode}
      </h3>
      <ChoiceCheckbox
        isSelected={on}
        onChange={(selected) => {
          void sync.preferences.set(DEVELOPER_MODE, selected);
        }}
        label={m.bots.developerMode}
        hint={m.bots.developerModeHint}
      />
      {on && <BotsDialog />}
    </section>
  );
}
