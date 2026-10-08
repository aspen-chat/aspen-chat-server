import { CheckIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { CheckboxButton, CheckboxField } from "react-aria-components";

/** A bordered choice (a radio button or checkbox) with its mark, label, and hint. */
export const choiceClass =
  "group flex items-start gap-2 rounded-md border border-line px-3 py-2 text-sm outline-none " +
  "hover:bg-surface-hover selected:border-accent disabled:opacity-60 disabled:hover:bg-transparent " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";

/** A choice as tall as the tallest of its fellows, which `useUniformHeight` measures. */
export const UNIFORM_CHOICE_CLASS = "min-h-(--choice-height)";

/** The square a checkbox's tick, or a radio button's dot, sits in. */
export const markClass =
  "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded border border-line bg-surface " +
  "text-accent-contrast group-selected:border-accent group-selected:bg-accent";

/** A checkbox drawn as a bordered choice with a label and, where it needs one, a hint. */
export function ChoiceCheckbox({
  isSelected,
  onChange,
  label,
  hint,
  isDisabled = false,
  className,
}: {
  isSelected: boolean;
  onChange: (selected: boolean) => void;
  label: string;
  hint?: string;
  isDisabled?: boolean;
  /** Added to the choice's own, to size it among others. */
  className?: string;
}) {
  return (
    <CheckboxField isSelected={isSelected} onChange={onChange} isDisabled={isDisabled}>
      <CheckboxButton
        className={className === undefined ? choiceClass : choiceClass + " " + className}
      >
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
          {hint !== undefined && <span className="text-xs text-ink-muted">{hint}</span>}
        </span>
      </CheckboxButton>
    </CheckboxField>
  );
}

/** A radio button's dot. */
export function RadioMark() {
  return (
    <span className={markClass + " rounded-full"}>
      <span className="forced-fill h-1.5 w-1.5 rounded-full bg-accent-contrast opacity-0 group-selected:opacity-100" />
    </span>
  );
}

/** A checkbox drawn small, as one of a list: its tick and its label, on one line. */
export function CompactCheckbox({
  isSelected,
  onChange,
  isDisabled = false,
  children,
}: {
  isSelected: boolean;
  onChange: (selected: boolean) => void;
  isDisabled?: boolean;
  children: ReactNode;
}) {
  return (
    <CheckboxField
      isSelected={isSelected}
      onChange={onChange}
      isDisabled={isDisabled}
      className="min-w-0"
    >
      <CheckboxButton className="group flex min-w-0 items-center gap-2 rounded px-2 py-1 text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 disabled:opacity-60 disabled:hover:bg-transparent">
        <span className={markClass + " mt-0"}>
          <CheckIcon
            size={12}
            weight="bold"
            aria-hidden="true"
            className="hidden group-selected:block"
          />
        </span>
        {children}
      </CheckboxButton>
    </CheckboxField>
  );
}
