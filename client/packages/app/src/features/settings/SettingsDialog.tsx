import {
  AUDIO_INPUT,
  AUDIO_OUTPUT,
  DEFAULT_DEVICE,
  NOTIFICATION_OUTPUT,
  SAME_AS_VOICE,
  VIDEO_INPUT,
  INPUT_VOLUME,
  OUTPUT_VOLUME,
  canChooseOutput,
} from "@aspen/protocol";
import { GearSixIcon, SignOutIcon } from "@phosphor-icons/react";
import { Button, Dialog, DialogTrigger, Modal, ModalOverlay } from "react-aria-components";
import { useSignOut } from "@/api/deploymentsContext";
import { AboutDialog } from "@/features/about/AboutDialog";
import { OtherServersSection } from "@/features/deployments/OtherServersSection";
import { usePreference, useSync, useVoiceCall } from "@/api/hooks";
import {
  columnPlanesModalClass,
  dialogClass,
  fullScreenOverlayClass,
  planeClass,
  secondaryButtonClass,
  touchFullScreenClass,
} from "@/features/invites/dialog";
import { Tooltip } from "@/features/layout/Tooltip";
import { SecurityDialog } from "@/features/security/SecurityDialog";
import { EmailDialog } from "@/features/email/EmailDialog";
import { BlockedUsersSection } from "@/features/settings/BlockedUsers";
import { DeveloperSection } from "@/features/settings/DeveloperSection";
import { FontsSection } from "@/features/settings/FontsSection";
import { LanguageSection } from "@/features/settings/LanguageSection";
import { NotificationsSection } from "@/features/settings/NotificationsSection";
import { PrivacySection } from "@/features/settings/PrivacySection";
import { AccessNotice, DeviceSelect } from "@/features/settings/DeviceSelect";
import { useAudioDevices } from "@/features/settings/useAudioDevices";
import { VolumeSlider } from "@/features/voice/VolumeSlider";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { PlaneColumns } from "@/features/layout/PlaneColumns";
import { useMessages } from "@/i18n/context";
import { ThemePicker } from "@/theme/ThemePicker";
import { NameColorsCheckbox } from "@/features/settings/NameColorsCheckbox";
import { MotionSpeedSlider } from "@/features/settings/MotionSpeedSlider";
import { MessageTextSizeSlider } from "@/features/settings/MessageTextSizeSlider";
import { MessageSpacingSlider } from "@/features/settings/MessageSpacingSlider";
import { AccessibilitySection } from "@/features/settings/AccessibilitySection";
import { TitleBarCheckbox } from "@/features/settings/TitleBarCheckbox";
import { ZoomSlider } from "@/features/settings/ZoomSlider";

/**
 * The user's preferences: first the account's sign-in and security settings and its email
 * address; then the microphone and speaker voice chat uses and the speaker for notification
 * sounds, kept with this install; whether this install notifies with the system and a sound; the
 * language, kept with the account; the appearance: the theme, palette, contrast, zoom, and whether
 * names take their roles' colours, kept with this install, and the message text size, line
 * spacing, and animation speed, kept with the account; whether new messages are read out, and the
 * fonts text and code are drawn in, kept with this install; whether others see the user typing,
 * kept with the account; the people the user has blocked; developer mode, with the user's bots;
 * and, beneath them, the way out of the account and the button that opens About Aspen. The small
 * planes follow Audio and video so they fill the first column beneath it rather than leave it
 * short. Sections for account-wide preferences slot in beside them.
 */
export function SettingsDialog({ triggerClassName }: { triggerClassName: string }) {
  const m = useMessages();
  const signOut = useSignOut();
  return (
    <DialogTrigger>
      <Tooltip text={m.settings.title}>
        <Button aria-label={m.settings.title} className={triggerClassName}>
          <GearSixIcon size={24} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay isDismissable className={fullScreenOverlayClass}>
        <Modal className={columnPlanesModalClass + " " + touchFullScreenClass}>
          <Dialog className={dialogClass}>
            {({ close }) => (
              <>
                <DialogHeading>{m.settings.title}</DialogHeading>
                <PlaneColumns>
                  <section aria-labelledby="settings-account" className={planeClass}>
                    <h3 id="settings-account" className="text-lg font-semibold text-ink-muted">
                      {m.settings.account}
                    </h3>
                    <SecurityDialog />
                    <EmailDialog />
                  </section>
                  <AudioSection />
                  <NotificationsSection />
                  <LanguageSection />
                  <section aria-labelledby="settings-appearance" className={planeClass}>
                    <h3 id="settings-appearance" className="text-lg font-semibold text-ink-muted">
                      {m.settings.appearance}
                    </h3>
                    <ThemePicker />
                    <ZoomSlider />
                    <MessageTextSizeSlider />
                    <MessageSpacingSlider />
                    <MotionSpeedSlider />
                    <NameColorsCheckbox />
                    <TitleBarCheckbox />
                  </section>
                  <AccessibilitySection />
                  <FontsSection />
                  <PrivacySection />
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
                  <AboutDialog onNavigate={close} />
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
  const sync = useSync();
  const inputVolume = usePreference(INPUT_VOLUME);
  const outputVolume = usePreference(OUTPUT_VOLUME);
  return (
    <section aria-labelledby="settings-audio" className={planeClass}>
      <h3 id="settings-audio" className="text-lg font-semibold text-ink-muted">
        {m.settings.audio}
      </h3>
      <AccessNotice kind="microphone" access={access.microphone} onAllow={requestAccess} />
      <DeviceSelect
        label={m.settings.microphone}
        definition={AUDIO_INPUT}
        devices={devices.inputs}
        defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
      />
      <VolumeSlider
        label={m.voice.inputVolume}
        gain={inputVolume}
        onChange={(value) => {
          void sync.preferences.set(INPUT_VOLUME, value);
        }}
      />
      <DeviceSelect
        label={m.settings.speaker}
        definition={AUDIO_OUTPUT}
        devices={outputs ? devices.outputs : []}
        defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
        disabled={!outputs}
        hint={outputs ? undefined : m.settings.outputUnsupported}
      />
      <VolumeSlider
        label={m.voice.outputVolume}
        gain={outputVolume}
        onChange={(value) => {
          void sync.preferences.set(OUTPUT_VOLUME, value);
        }}
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
