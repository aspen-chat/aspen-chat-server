import {
  resolveDevice,
  type DeviceChoice,
  type NotificationChoice,
  type PreferenceDefinition,
} from "@aspen/protocol";
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
import { usePreference, useSync } from "@/api/hooks";
import {
  optionClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { type AudioDevice } from "@/features/settings/audioDevices";
import { type DeviceAccess, type DeviceKind } from "@/features/settings/useAudioDevices";
import { useMessages } from "@/i18n/context";

/**
 * What stands in for a list of devices the browser will not name yet: a button that asks for the
 * permission while it can still be asked, or what to do once it was refused.
 */
export function AccessNotice({
  kind,
  access,
  onAllow,
}: {
  kind: DeviceKind;
  access: DeviceAccess;
  onAllow: (kind: DeviceKind) => Promise<void>;
}) {
  const m = useMessages();
  const microphone = kind === "microphone";
  switch (access) {
    case "granted":
      return null;
    case "denied":
      return (
        <p className="text-sm text-ink-muted">
          {microphone ? m.settings.microphoneDenied : m.settings.cameraDenied}
        </p>
      );
    case "absent":
      return (
        <p className="text-sm text-ink-muted">
          {microphone ? m.settings.noMicrophone : m.settings.noCamera}
        </p>
      );
    case "locked":
      return (
        <div className="flex flex-col items-start gap-1.5">
          <p className="text-sm text-ink-muted">
            {microphone ? m.settings.microphoneLocked : m.settings.cameraLocked}
          </p>
          <Button
            onPress={() => {
              void onAllow(kind);
            }}
            className={secondaryButtonClass}
          >
            {microphone ? m.settings.allowMicrophone : m.settings.allowCamera}
          </Button>
        </div>
      );
  }
}

const MISSING = "missing";

/** Which device of a kind the preference `definition` names, chosen from those listed. */
export function DeviceSelect({
  label,
  definition,
  devices,
  defaults,
  disabled,
  hint,
}: {
  label: string;
  definition: PreferenceDefinition<NotificationChoice>;
  devices: readonly AudioDevice[];
  /** The choices that are not devices: the system default, and "same as voice chat". */
  defaults: readonly AudioDevice[];
  disabled?: boolean | undefined;
  hint?: string | undefined;
}) {
  const m = useMessages();
  const sync = useSync();
  const chosen = usePreference(definition);
  // The remembered device is found by id or by label; one that is not plugged in right now
  // stays selected under a "missing" entry so the choice is not lost.
  const selectedKey =
    typeof chosen === "string"
      ? chosen
      : (resolveDevice(
          chosen,
          devices.map((d) => ({ deviceId: d.id, label: d.label })),
        ) ?? MISSING);
  const known = [...defaults, ...devices];
  const options =
    selectedKey === MISSING ? [...known, { id: MISSING, label: m.settings.missingDevice }] : known;
  return (
    <div className="flex flex-col gap-1">
      <Select
        value={selectedKey}
        onChange={(key) => {
          if (typeof key !== "string" || key === MISSING) {
            return;
          }
          const device = devices.find((d) => d.id === key);
          const choice: NotificationChoice =
            device === undefined ? (key as DeviceChoice) : { id: device.id, label: device.label };
          void sync.preferences.set(definition, choice);
        }}
        isDisabled={disabled ?? false}
        className="flex flex-col gap-1"
      >
        <Label className="text-sm font-medium">{label}</Label>
        <Button className={selectButtonClass}>
          <SelectValue className="truncate" />
          <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
        </Button>
        <Popover className={selectPopoverClass}>
          <ListBox items={options}>
            {(device) => (
              <ListBoxItem id={device.id} textValue={device.label} className={optionClass}>
                {device.label}
              </ListBoxItem>
            )}
          </ListBox>
        </Popover>
      </Select>
      {hint !== undefined && <p className="text-xs text-ink-muted">{hint}</p>}
    </div>
  );
}
