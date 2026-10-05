import { useState } from "react";
import {
  CaretDownIcon,
  CircleHalfIcon,
  CircleHalfTiltIcon,
  DesktopIcon,
  MoonIcon,
  SunIcon,
  type Icon,
} from "@phosphor-icons/react";
import {
  Button,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  RadioButton,
  RadioField,
  RadioGroup,
  Select,
  SelectValue,
  Text,
} from "react-aria-components";
import { useMessages } from "@/i18n/context";
import {
  CONTRAST_MODES,
  PALETTES,
  THEME_MODES,
  applyContrastMode,
  applyPalette,
  applyThemeMode,
  isContrastMode,
  isPalette,
  isThemeMode,
  storedContrastMode,
  storedPalette,
  storedThemeMode,
  type ContrastMode,
  type Palette,
  type ThemeMode,
} from "./palettes";
import { selectPopoverClass } from "@/features/invites/dialog";

const MODE_ICONS: Record<ThemeMode, Icon> = {
  system: DesktopIcon,
  light: SunIcon,
  dark: MoonIcon,
};

const CONTRAST_ICONS: Record<ContrastMode, Icon> = {
  system: DesktopIcon,
  standard: CircleHalfIcon,
  more: CircleHalfTiltIcon,
};

/**
 * Lets the user choose whether Aspen is drawn light or dark or as their system prefers, its
 * colour palette, and its contrast, in the settings dialog. Each applies at once and persists
 * with the install.
 */
export function ThemePicker() {
  return (
    <>
      <ModePicker />
      <PalettePicker />
      <ContrastPicker />
    </>
  );
}

function ModePicker() {
  const m = useMessages();
  const [mode, setMode] = useState<ThemeMode>(storedThemeMode);
  return (
    <Segments
      label={m.themeModeLabel}
      values={THEME_MODES}
      value={mode}
      icons={MODE_ICONS}
      names={m.themeModes}
      onChange={(value) => {
        if (isThemeMode(value)) {
          setMode(value);
          applyThemeMode(value);
        }
      }}
    />
  );
}

function ContrastPicker() {
  const m = useMessages();
  const [mode, setMode] = useState<ContrastMode>(storedContrastMode);
  return (
    <Segments
      label={m.contrastLabel}
      values={CONTRAST_MODES}
      value={mode}
      icons={CONTRAST_ICONS}
      names={m.contrastModes}
      hint={m.contrastHint}
      onChange={(value) => {
        if (isContrastMode(value)) {
          setMode(value);
          applyContrastMode(value);
        }
      }}
    />
  );
}

/** A choice of a few, side by side, each with its icon and name. */
function Segments<T extends string>({
  label,
  values,
  value,
  icons,
  names,
  hint,
  onChange,
}: {
  label: string;
  values: readonly T[];
  value: T;
  icons: Record<T, Icon>;
  names: Record<T, string>;
  hint?: string;
  onChange: (value: string) => void;
}) {
  return (
    <RadioGroup
      value={value}
      onChange={onChange}
      orientation="horizontal"
      className="flex flex-col gap-1"
    >
      <Label className="text-sm font-medium">{label}</Label>
      <div className="grid grid-cols-3 gap-1 rounded-md border border-line bg-surface p-0.5">
        {values.map((option) => {
          const OptionIcon: Icon = icons[option];
          return (
            <RadioField key={option} value={option}>
              <RadioButton className="flex cursor-default items-center justify-center gap-1.5 rounded px-2 py-1.5 text-sm whitespace-nowrap text-ink-muted outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 selected:bg-surface-raised selected:font-medium selected:text-accent selected:shadow-sm">
                <OptionIcon size={16} aria-hidden="true" />
                {names[option]}
              </RadioButton>
            </RadioField>
          );
        })}
      </div>
      {hint !== undefined && (
        <Text slot="description" className="text-xs text-ink-muted">
          {hint}
        </Text>
      )}
    </RadioGroup>
  );
}

function PalettePicker() {
  const m = useMessages();
  const [palette, setPalette] = useState<Palette>(storedPalette);
  return (
    <Select
      value={palette}
      onChange={(key) => {
        if (isPalette(key)) {
          setPalette(key);
          applyPalette(key);
        }
      }}
      className="flex flex-col gap-1"
    >
      <Label className="text-sm font-medium">{m.paletteLabel}</Label>
      <Button className="flex w-full items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 py-2 text-start text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent/50">
        <SelectValue className="truncate" />
        <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
      </Button>
      <Popover className={selectPopoverClass}>
        <ListBox className="outline-none">
          {PALETTES.map((name) => (
            <ListBoxItem
              key={name}
              id={name}
              textValue={m.palettes[name]}
              className="cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover selected:font-medium selected:text-accent"
            >
              {m.palettes[name]}
            </ListBoxItem>
          ))}
        </ListBox>
      </Popover>
    </Select>
  );
}
