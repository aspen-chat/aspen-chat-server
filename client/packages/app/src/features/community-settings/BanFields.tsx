import { CaretDownIcon } from "@phosphor-icons/react";
import {
  Button,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
  TextField,
} from "react-aria-components";
import { fieldClass, hintClass, inputClass, labelClass } from "@/features/auth/styles";
import { optionClass, selectButtonClass, selectPopoverClass } from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { DELETE_WINDOWS, DURATIONS, type BanChoice } from "@/features/community-settings/banChoice";

/**
 * A ban's reason, how long it lasts, and, where the banner may delete messages, how far back
 * the person's go with it: the fields a community's ban, a ban from the server, and a report's
 * ban share. `reasonHint` and `deleteLabel` say what this ban means by them.
 */
export function BanFields({
  value,
  onChange,
  mayDelete,
  reasonHint,
  deleteLabel,
}: {
  value: BanChoice;
  onChange: (value: BanChoice) => void;
  mayDelete: boolean;
  reasonHint: string;
  deleteLabel: string;
}) {
  const m = useMessages();
  return (
    <>
      <TextField
        value={value.reason}
        onChange={(reason) => {
          onChange({ ...value, reason });
        }}
        maxLength={512}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.members.banReasonLabel}</Label>
        <Input className={inputClass} />
        <p className={hintClass}>{reasonHint}</p>
      </TextField>
      <Select
        value={value.duration}
        onChange={(key) => {
          const found = DURATIONS.find((d) => d.id === key);
          if (found !== undefined) {
            onChange({ ...value, duration: found.id });
          }
        }}
        className="flex flex-col gap-1"
      >
        <Label className={labelClass}>{m.members.banDurationLabel}</Label>
        <Button className={selectButtonClass}>
          <SelectValue className="truncate" />
          <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
        </Button>
        <Popover className={selectPopoverClass}>
          <ListBox>
            {DURATIONS.map((d) => (
              <ListBoxItem
                key={d.id}
                id={d.id}
                textValue={m.members.banDurations[d.id]}
                className={optionClass}
              >
                {m.members.banDurations[d.id]}
              </ListBoxItem>
            ))}
          </ListBox>
        </Popover>
      </Select>
      {mayDelete && (
        <Select
          value={value.window}
          onChange={(key) => {
            const found = DELETE_WINDOWS.find((w) => w.id === key);
            if (found !== undefined) {
              onChange({ ...value, window: found.id });
            }
          }}
          className="flex flex-col gap-1"
        >
          <Label className={labelClass}>{deleteLabel}</Label>
          <Button className={selectButtonClass}>
            <SelectValue className="truncate" />
            <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
          </Button>
          <Popover className={selectPopoverClass}>
            <ListBox>
              {DELETE_WINDOWS.map((w) => (
                <ListBoxItem
                  key={w.id}
                  id={w.id}
                  textValue={m.members.banDeleteOptions[w.id]}
                  className={optionClass}
                >
                  {m.members.banDeleteOptions[w.id]}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
      )}
    </>
  );
}
