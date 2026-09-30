import { useState } from "react";
import { CaretDownIcon, DesktopIcon, MoonIcon, SunIcon, type Icon } from "@phosphor-icons/react";
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
} from "react-aria-components";
import { useMessages } from "@/i18n/context";
import {
  PALETTES,
  THEME_MODES,
  applyPalette,
  applyThemeMode,
  isPalette,
  isThemeMode,
  storedPalette,
  storedThemeMode,
  type Palette,
  type ThemeMode,
} from "./palettes";
import { selectPopoverClass } from "@/features/invites/dialog";

const MODE_ICONS: Record<ThemeMode, Icon> = {
  system: DesktopIcon,
  light: SunIcon,
  dark: MoonIcon,
};

/**
 * Lets the user choose whether Aspen is drawn light or dark or as their system prefers, and
 * its colour palette, in the settings dialog. Both apply at once and persist with the install.
 */
export function ThemePicker() {
  return (
    <>
      <ModePicker />
      <PalettePicker />
    </>
  );
}

function ModePicker() {
  const m = useMessages();
  const [mode, setMode] = useState<ThemeMode>(storedThemeMode);
  return (
    <RadioGroup
      value={mode}
      onChange={(value) => {
        if (isThemeMode(value)) {
          setMode(value);
          applyThemeMode(value);
        }
      }}
      orientation="horizontal"
      className="flex flex-col gap-1"
    >
      <Label className="text-sm font-medium">{m.themeModeLabel}</Label>
      <div className="grid grid-cols-3 gap-1 rounded-md border border-line bg-surface p-0.5">
        {THEME_MODES.map((value) => {
          const ModeIcon = MODE_ICONS[value];
          return (
            <RadioField key={value} value={value}>
              <RadioButton className="flex cursor-default items-center justify-center gap-1.5 rounded px-2 py-1.5 text-sm whitespace-nowrap text-ink-muted outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 selected:bg-surface-raised selected:font-medium selected:text-accent selected:shadow-sm">
                <ModeIcon size={16} aria-hidden="true" />
                {m.themeModes[value]}
              </RadioButton>
            </RadioField>
          );
        })}
      </div>
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
