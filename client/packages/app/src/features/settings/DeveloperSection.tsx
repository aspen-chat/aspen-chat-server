import { DEVELOPER_MODE, ID_WIZARD } from "@aspen/protocol";
import { usePreference, useSync } from "@/api/hooks";
import { BotsDialog } from "@/features/bots/BotsDialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";
import { planeClass } from "@/features/invites/dialog";

/** The opt-in to developer mode, and, once in it, the ID wizard and the user's bots. */
export function DeveloperSection() {
  const m = useMessages();
  const sync = useSync();
  const on = usePreference(DEVELOPER_MODE);
  const wizard = usePreference(ID_WIZARD);
  return (
    <section aria-labelledby="settings-developer" className={planeClass}>
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
      {on && (
        <ChoiceCheckbox
          isSelected={wizard}
          onChange={(selected) => {
            void sync.preferences.set(ID_WIZARD, selected);
          }}
          label={m.bots.idWizard}
          hint={m.bots.idWizardHint}
        />
      )}
      {on && <BotsDialog />}
    </section>
  );
}
