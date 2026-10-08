import type { Role } from "@aspen/protocol";
import { Label, RadioButton, RadioField, RadioGroup } from "react-aria-components";
import { fieldClass, labelClass } from "@/features/auth/styles";
import type { Preset, PresetOption } from "@/features/community-settings/accessPresets";
import { CompactCheckbox, RadioMark, choiceClass } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";

/**
 * The settings as radio choices and, for the two that name roles, a checkbox for each of
 * `roles` (highest first, everyone's left out by the caller).
 */
export function PresetChoices({
  options,
  preset,
  onPresetChange,
  roles,
  chosen,
  onChosenChange,
}: {
  options: readonly PresetOption[];
  preset: Preset;
  onPresetChange: (preset: Preset) => void;
  roles: readonly Role[];
  chosen: ReadonlySet<string>;
  onChosenChange: (chosen: ReadonlySet<string>) => void;
}) {
  const m = useMessages();
  return (
    <>
      <RadioGroup
        value={preset}
        onChange={(value) => {
          onPresetChange(value as Preset);
        }}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.access.presetsLabel}</Label>
        <div className="grid gap-2 sm:grid-cols-2">
          {options.map((option) => (
            <RadioField key={option.key} value={option.key}>
              <RadioButton className={choiceClass}>
                <RadioMark />
                <span className="flex flex-col">
                  <span className="font-medium">{option.label}</span>
                  <span className="text-xs text-ink-muted">{option.hint}</span>
                </span>
              </RadioButton>
            </RadioField>
          ))}
        </div>
      </RadioGroup>
      {(preset === "private" || preset === "readOnly") && (
        <fieldset className="flex flex-col gap-1">
          <legend className={labelClass}>{m.access.rolesLabel}</legend>
          {roles.map((role) => (
            <CompactCheckbox
              key={role.id}
              isSelected={chosen.has(role.id)}
              onChange={(selected) => {
                const next = new Set(chosen);
                if (selected) {
                  next.add(role.id);
                } else {
                  next.delete(role.id);
                }
                onChosenChange(next);
              }}
            >
              {role.name}
            </CompactCheckbox>
          ))}
        </fieldset>
      )}
    </>
  );
}
