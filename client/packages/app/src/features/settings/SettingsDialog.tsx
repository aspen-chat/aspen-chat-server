import {
  AUDIO_INPUT,
  AUDIO_OUTPUT,
  DEFAULT_DEVICE,
  NOTIFICATION_OUTPUT,
  SAME_AS_VOICE,
  VIDEO_INPUT,
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
import { useSignOut } from "@/api/deploymentsContext";
import { OtherServersSection } from "@/features/deployments/OtherServersSection";
import { usePreference, useSync, useVoiceCall } from "@/api/hooks";
import {
  dialogClass,
  optionClass,
  overlayClass,
  planeClass,
  widePlanesModalClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { Tooltip } from "@/features/layout/Tooltip";
import { SecurityDialog } from "@/features/security/SecurityDialog";
import { BlockedUsersSection } from "@/features/settings/BlockedUsers";
import { DeveloperSection } from "@/features/settings/DeveloperSection";
import { FontsSection } from "@/features/settings/FontsSection";
import { LanguageSection } from "@/features/settings/LanguageSection";
import { NotificationsSection } from "@/features/settings/NotificationsSection";
import { type AudioDevice } from "@/features/settings/audioDevices";
import {
  type DeviceAccess,
  type DeviceKind,
  useAudioDevices,
} from "@/features/settings/useAudioDevices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { PlaneColumns } from "@/features/layout/PlaneColumns";
import { useMessages } from "@/i18n/context";
import { ThemePicker } from "@/theme/ThemePicker";
import { NameColorsCheckbox } from "@/features/settings/NameColorsCheckbox";
import { MotionSpeedSlider } from "@/features/settings/MotionSpeedSlider";

/**
 * The user's preferences: first the account's sign-in and security settings; then the
 * microphone and speaker voice chat uses and the speaker for notification sounds, the
 * appearance, and the fonts text and code are drawn in, all kept with this install; the language, kept with the account; the people the
 * user has blocked; developer mode, with the user's bots; and the way out of the account. Sections for
 * account-wide preferences slot in beside them.
 */
export function SettingsDialog({ triggerClassName }: { triggerClassName: string }) {
  const m = useMessages();
  const signOut = useSignOut();
  return (
    <DialogTrigger>
      <Tooltip text={m.settings.title}>
        <Button aria-label={m.settings.title} className={triggerClassName}>
          <GearSixIcon size={16} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={widePlanesModalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => (
              <>
                <DialogHeading>{m.settings.title}</DialogHeading>
                <PlaneColumns>
                  <section aria-labelledby="settings-account" className={planeClass}>
                    <h3 id="settings-account" className="text-sm font-semibold text-ink-muted">
                      {m.settings.account}
                    </h3>
                    <SecurityDialog />
                  </section>
                  <AudioSection />
                  <section aria-labelledby="settings-appearance" className={planeClass}>
                    <h3 id="settings-appearance" className="text-sm font-semibold text-ink-muted">
                      {m.settings.appearance}
                    </h3>
                    <ThemePicker />
                    <MotionSpeedSlider />
                    <NameColorsCheckbox />
                  </section>
                  <FontsSection />
                  <NotificationsSection />
                  <LanguageSection />
                  <BlockedUsersSection />
                  <OtherServersSection />
                  <DeveloperSection />
                </PlaneColumns>
                <div className="flex items-center justify-between gap-2">
                  <Button
                    onPress={() => {
                      close();
                      void signOut();
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
  const inCall = useVoiceCall().status === "connected";
  const { devices, access, requestAccess } = useAudioDevices(inCall);
  const outputs = canChooseOutput();
  return (
    <section aria-labelledby="settings-audio" className={planeClass}>
      <h3 id="settings-audio" className="text-sm font-semibold text-ink-muted">
        {m.settings.audio}
      </h3>
      <AccessNotice kind="microphone" access={access.microphone} onAllow={requestAccess} />
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
      <AccessNotice kind="camera" access={access.camera} onAllow={requestAccess} />
      <DeviceSelect
        label={m.settings.camera}
        definition={VIDEO_INPUT}
        devices={devices.cameras}
        defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
      />
    </section>
  );
}

/**
 * What stands in for a list of devices the browser will not name yet: a button that asks for the
 * permission while it can still be asked, or what to do once it was refused.
 */
function AccessNotice({
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
