import {
  AUDIO_INPUT,
  AUDIO_OUTPUT,
  DEFAULT_DEVICE,
  NOTIFICATION_OUTPUT,
  SAME_AS_VOICE,
  canChooseOutput,
  resolveDevice,
  type DeviceChoice,
  type NotificationChoice,
  type PreferenceDefinition,
} from "@aspen/protocol";
import { CaretDownIcon, GearSixIcon, SignOutIcon } from "@phosphor-icons/react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { usePreference, useSync } from "@/api/hooks";
import {
  dialogClass,
  modalClass,
  optionClass,
  overlayClass,
  secondaryButtonClass,
  selectButtonClass,
} from "@/features/invites/dialog";
import { Tooltip } from "@/features/layout/Tooltip";
import { SecurityDialog } from "@/features/security/SecurityDialog";
import { BlockedUsersSection } from "@/features/settings/BlockedUsers";
import { type AudioDevice } from "@/features/settings/audioDevices";
import { useAudioDevices } from "@/features/settings/useAudioDevices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { ThemePicker } from "@/theme/ThemePicker";

/**
 * The user's preferences: the microphone and speaker voice chat uses and the speaker for
 * notification sounds, then the colour palette, all kept with this install; the people the
 * user has blocked; the account's sign-in and security settings; and the way out of the
 * account. Sections for account-wide
 * preferences slot in beside them.
 */
export function SettingsDialog({ triggerClassName }: { triggerClassName: string }) {
  const m = useMessages();
  const client = useAspenClient();
  return (
    <DialogTrigger>
      <Tooltip text={m.settings.title}>
        <Button aria-label={m.settings.title} className={triggerClassName}>
          <GearSixIcon size={16} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => (
              <>
                <DialogHeading>{m.settings.title}</DialogHeading>
                <AudioSection />
                <section aria-labelledby="settings-appearance" className="flex flex-col gap-3">
                  <h3 id="settings-appearance" className="text-sm font-semibold text-ink-muted">
                    {m.settings.appearance}
                  </h3>
                  <ThemePicker />
                </section>
                <BlockedUsersSection />
                <section aria-labelledby="settings-account" className="flex flex-col gap-3">
                  <h3 id="settings-account" className="text-sm font-semibold text-ink-muted">
                    {m.settings.account}
                  </h3>
                  <SecurityDialog />
                </section>
                <div className="flex items-center justify-between gap-2">
                  <Button
                    onPress={() => {
                      close();
                      void client.logout();
                    }}
                    className={secondaryButtonClass + " flex items-center gap-1.5 text-danger"}
                  >
                    <SignOutIcon size={16} aria-hidden="true" />
                    {m.signOut}
                  </Button>
                </div>
              </>
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function AudioSection() {
  const m = useMessages();
  const { devices, permission } = useAudioDevices();
  const outputs = canChooseOutput();
  return (
    <section aria-labelledby="settings-audio" className="flex flex-col gap-3">
      <h3 id="settings-audio" className="text-sm font-semibold text-ink-muted">
        {m.settings.audio}
      </h3>
      {permission === "denied" && (
        <p className="text-sm text-ink-muted">{m.settings.microphoneDenied}</p>
      )}
      <DeviceSelect
        label={m.settings.microphone}
        definition={AUDIO_INPUT}
        devices={devices.inputs}
        defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
      />
      <DeviceSelect
        label={m.settings.speaker}
        definition={AUDIO_OUTPUT}
        devices={outputs ? devices.outputs : []}
        defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
        disabled={!outputs}
        hint={outputs ? undefined : m.settings.outputUnsupported}
      />
      <DeviceSelect
        label={m.settings.notificationOutput}
        definition={NOTIFICATION_OUTPUT}
        devices={outputs ? devices.outputs : []}
        defaults={[
          { id: SAME_AS_VOICE, label: m.settings.sameAsVoice },
          { id: DEFAULT_DEVICE, label: m.settings.systemDefault },
        ]}
        disabled={!outputs}
      />
    </section>
  );
}

const MISSING = "missing";

function DeviceSelect({
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
        <Popover className="min-w-(--trigger-width) rounded-md border border-line bg-surface-raised p-1 shadow-lg">
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
