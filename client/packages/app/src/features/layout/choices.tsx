import { CheckIcon } from "@phosphor-icons/react";
import { CheckboxButton, CheckboxField } from "react-aria-components";

/** A bordered choice (a radio button or checkbox) with its mark, label, and hint. */
export const choiceClass =
  "group flex items-start gap-2 rounded-md border border-line px-3 py-2 text-sm outline-none " +
  "hover:bg-surface-hover selected:border-accent disabled:opacity-60 disabled:hover:bg-transparent " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";

/** The square a checkbox's tick, or a radio button's dot, sits in. */
export const markClass =
  "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border border-line bg-surface " +
  "text-accent-contrast group-selected:border-accent group-selected:bg-accent";

/** A checkbox drawn as a bordered choice with a label and a hint. */
export function ChoiceCheckbox({
  isSelected,
  onChange,
  label,
  hint,
  isDisabled = false,
}: {
  isSelected: boolean;
  onChange: (selected: boolean) => void;
  label: string;
  hint: string;
  isDisabled?: boolean;
}) {
  return (
    <CheckboxField isSelected={isSelected} onChange={onChange} isDisabled={isDisabled}>
      <CheckboxButton className={choiceClass}>
        <span className={markClass}>
          <CheckIcon
            size={12}
            weight="bold"
            aria-hidden="true"
            className="hidden group-selected:block"
          />
        </span>
        <span className="flex flex-col">
          <span className="font-medium">{label}</span>
          <span className="text-xs text-ink-muted">{hint}</span>
        </span>
      </CheckboxButton>
    </CheckboxField>
  );
}

/** A radio button's dot. */
export function RadioMark() {
  return (
    <span className={markClass + " rounded-full"}>
      <span className="h-1.5 w-1.5 rounded-full bg-accent-contrast opacity-0 group-selected:opacity-100" />
    </span>
  );
}
