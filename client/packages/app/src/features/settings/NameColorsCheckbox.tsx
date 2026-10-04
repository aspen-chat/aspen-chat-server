import { NAME_COLORS } from "@aspen/protocol";
import { usePreference, useSync } from "@/api/hooks";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";

/** Whether this install draws people's names in their roles' colours. */
export function NameColorsCheckbox() {
  const m = useMessages();
  const sync = useSync();
  const shown = usePreference(NAME_COLORS);
  return (
    <ChoiceCheckbox
      isSelected={shown}
      onChange={(selected) => {
        void sync.preferences.set(NAME_COLORS, selected);
      }}
      label={m.settings.nameColors}
      hint={m.settings.nameColorsHint}
    />
  );
}
