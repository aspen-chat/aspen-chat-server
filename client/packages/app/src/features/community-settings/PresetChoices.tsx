import type { Role } from "@aspen/protocol";
import { CheckIcon } from "@phosphor-icons/react";
import {
  CheckboxButton,
  CheckboxField,
  Label,
  RadioButton,
  RadioField,
  RadioGroup,
} from "react-aria-components";
import { fieldClass, labelClass } from "@/features/auth/styles";
import type { Preset, PresetOption } from "@/features/community-settings/accessPresets";
import { RadioMark, choiceClass, markClass } from "@/features/layout/choices";
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
            <CheckboxField
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
              <CheckboxButton className="group flex items-center gap-2 rounded px-2 py-1 text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50">
                <span className={markClass + " mt-0"}>
                  <CheckIcon
                    size={12}
                    weight="bold"
                    aria-hidden="true"
                    className="hidden group-selected:block"
                  />
                </span>
                {role.name}
              </CheckboxButton>
            </CheckboxField>
          ))}
        </fieldset>
      )}
    </>
  );
}
