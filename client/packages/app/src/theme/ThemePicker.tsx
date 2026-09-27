import { useState } from "react";
import { CaretDownIcon } from "@phosphor-icons/react";
import {
  Button,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import { useMessages } from "@/i18n/context";
import { PALETTES, applyPalette, isPalette, storedPalette, type Palette } from "./palettes";

/**
 * Lets the user switch the colour palette, as a labelled select in the settings dialog. The
 * choice applies at once and persists with the install.
 */
export function ThemePicker() {
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
      <Button className="flex w-full items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 py-2 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent/50">
        <SelectValue className="truncate" />
        <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
      </Button>
      <Popover className="min-w-(--trigger-width) rounded-md border border-line bg-surface-raised p-1 shadow-lg">
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
