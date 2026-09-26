import { useState } from "react";
import { Button, ListBox, ListBoxItem, Popover, Select, SelectValue } from "react-aria-components";
import { useMessages } from "@/i18n/context";
import { PALETTES, applyPalette, isPalette, storedPalette, type Palette } from "./palettes";

/** Lets the user switch the colour palette. The choice applies at once and persists. */
export function ThemePicker() {
  const m = useMessages();
  const [palette, setPalette] = useState<Palette>(storedPalette);
  return (
    <Select
      aria-label={m.paletteLabel}
      value={palette}
      onChange={(key) => {
        if (isPalette(key)) {
          setPalette(key);
          applyPalette(key);
        }
      }}
      className="flex flex-col"
    >
      <Button className="rounded-md border border-line bg-surface-raised px-2 py-1 text-xs text-ink-muted outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50">
        <SelectValue />
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
