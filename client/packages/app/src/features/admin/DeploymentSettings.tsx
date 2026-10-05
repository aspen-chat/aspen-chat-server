import type { DeploymentSettings, DeploymentSettingsUpdateRequest } from "@aspen/protocol";
import { useCallback, useState } from "react";
import { Button, Form, Input, Label, NumberField, Text } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { useAdminRead } from "@/features/admin/useAdminRead";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { LoadingLabel } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";

/** The largest count the server keeps, as `INTEGER`. */
const MAX_COUNT = 2_147_483_647;

type Toggle =
  | "registrationInviteRequired"
  | "requireTwoFactor"
  | "botsEnabled"
  | "fileTransfers"
  | "emailRequired"
  | "emailVerificationRequired"
  | "newsletterEnabled";
type Count = "botsMaxPerUser" | "everyoneMentionLimit" | "customEmojiLimit";

/**
 * The deployment's policies, for holders of Manage deployment settings: whether registering
 * takes an invite, whether every account needs a second factor, email (an address to register, a
 * verified one to use the server, and a newsletter, each offered only where the server can send
 * mail), bots, the limits on communities, and files in calls. A change reaches every server at
 * once.
 */
export function DeploymentSettingsSection() {
  const m = useMessages();
  const sync = useSync();
  const load = useCallback(() => sync.admin.deploymentSettings(), [sync]);
  const settings = useAdminRead(load);
  return (
    <Section id="deployment-policies" title={m.admin.policiesTitle} hint={m.admin.policiesHint}>
      {settings.error !== null && <ReadFailed error={settings.error} onRetry={settings.reload} />}
      {settings.data === undefined && settings.error === null && <LoadingLabel />}
      {settings.data !== undefined && <PoliciesForm initial={settings.data} />}
    </Section>
  );
}

/**
 * The policies' fields, starting from what the server has; saving sends only what changed, and
 * the server's answer is what the fields then start from.
 */
function PoliciesForm({ initial }: { initial: DeploymentSettings }) {
  const m = useMessages();
  const sync = useSync();
  const [saved, setSaved] = useState(initial);
  const [draft, setDraft] = useState(initial);
  const [saving, setSaving] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const change: DeploymentSettingsUpdateRequest = Object.fromEntries(
    (Object.keys(draft) as (keyof DeploymentSettings)[])
      .filter((key) => draft[key] !== saved[key])
      .map((key) => [key, draft[key]]),
  );
  const changed = Object.keys(change).length > 0;

  function set<K extends keyof DeploymentSettings>(key: K, value: DeploymentSettings[K]) {
    setDraft((d) => ({ ...d, [key]: value }));
    setDone(false);
  }

  const toggle = (key: Toggle, label: string, hint?: string, isDisabled = false) => (
    <ChoiceCheckbox
      isSelected={draft[key]}
      onChange={(value) => {
        set(key, value);
      }}
      label={label}
      isDisabled={isDisabled}
      {...(hint === undefined ? {} : { hint })}
    />
  );
  // Without mail, what needs it cannot be turned on; one already on can still be turned off.
  const noMail = !draft.emailAvailable;
  const emailToggle = (key: Toggle, label: string, hint: string) =>
    toggle(key, label, hint, noMail && !draft[key]);
  const count = (key: Count, label: string, hint?: string) => (
    <NumberField
      value={draft[key]}
      onChange={(value) => {
        set(key, Number.isNaN(value) ? saved[key] : value);
      }}
      minValue={0}
      maxValue={MAX_COUNT}
      formatOptions={{ maximumFractionDigits: 0 }}
      className={fieldClass}
    >
      <Label className={labelClass}>{label}</Label>
      <Input className={inputClass + " sm:max-w-40"} />
      {hint !== undefined && (
        <Text slot="description" className={hintClass}>
          {hint}
        </Text>
      )}
    </NumberField>
  );

  return (
    <Form
      aria-label={m.admin.policiesTitle}
      onSubmit={(event) => {
        event.preventDefault();
        setSaving(true);
        setError(null);
        void sync.admin
          .updateDeploymentSettings(change)
          .then((now) => {
            setSaved(now);
            setDraft(now);
            setDone(true);
          })
          .catch((e: unknown) => {
            setError(problemText(e));
          })
          .finally(() => {
            setSaving(false);
          });
      }}
      className="flex flex-col gap-4"
    >
      <div className="flex flex-col gap-2">
        {toggle(
          "registrationInviteRequired",
          m.admin.registrationInviteRequired,
          m.admin.registrationInviteRequiredHint,
        )}
        {toggle("requireTwoFactor", m.admin.requireTwoFactor, m.admin.requireTwoFactorHint)}
        {toggle("botsEnabled", m.admin.botsEnabled, m.admin.botsEnabledHint)}
        {toggle("fileTransfers", m.admin.fileTransfersAllowed, m.admin.fileTransfersAllowedHint)}
      </div>
      <fieldset className="flex flex-col gap-2">
        <legend className="mb-2 text-sm font-semibold text-ink-muted">
          {m.admin.emailHeading}
        </legend>
        {noMail && <p className={hintClass}>{m.admin.emailUnavailable}</p>}
        {emailToggle("emailRequired", m.admin.emailRequired, m.admin.emailRequiredHint)}
        {emailToggle(
          "emailVerificationRequired",
          m.admin.emailVerificationRequired,
          m.admin.emailVerificationRequiredHint,
        )}
        {emailToggle("newsletterEnabled", m.admin.newsletterEnabled, m.admin.newsletterEnabledHint)}
      </fieldset>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        {count("botsMaxPerUser", m.admin.botsMaxPerUser)}
        {count(
          "everyoneMentionLimit",
          m.admin.everyoneMentionLimit,
          m.admin.everyoneMentionLimitHint,
        )}
        {count("customEmojiLimit", m.admin.customEmojiLimit)}
      </div>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className="flex items-center gap-3">
        <Button type="submit" isDisabled={saving || !changed} className={primaryButtonClass}>
          {saving ? m.admin.savingSettings : m.admin.saveSettings}
        </Button>
        {done && (
          <span role="status" className="text-sm text-ink-muted">
            {m.admin.settingsSaved}
          </span>
        )}
      </div>
    </Form>
  );
}
