import type { DeploymentProfileUpdateRequest } from "@aspen/protocol";
import { useCallback, useState } from "react";
import { Button, Form, Input, Label, Text, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
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
import { secondaryButtonClass } from "@/features/invites/dialog";
import { IconPicker } from "@/features/media/IconPicker";
import { useMessages } from "@/i18n/context";

/** The longest display name, as the server's `DISPLAY_NAME_MAX_CHARS`. */
const DISPLAY_NAME_MAX_CHARS = 64;

/**
 * How the deployment presents itself, for holders of Manage deployment settings: the display
 * name and icon its sign-in screen welcomes people with (`DeploymentWelcome`), which also names
 * its system account and what authenticators call it. An empty name is saved as none, which
 * welcomes people without one.
 */
export function DeploymentProfileSection() {
  const m = useMessages();
  const client = useAspenClient();
  const sync = useSync();
  const load = useCallback(() => client.deploymentProfile(), [client]);
  const profile = useAdminRead(load);
  const [iconError, setIconError] = useState<string | null>(null);

  /** Saves a change and reads the profile again; whether it was saved. */
  async function update(
    change: DeploymentProfileUpdateRequest,
    fail: (e: string) => void,
  ): Promise<boolean> {
    try {
      await sync.admin.updateDeploymentProfile(change);
      profile.reload();
      return true;
    } catch (e) {
      fail(problemText(e));
      return false;
    }
  }

  const icon = profile.data?.icon ?? null;
  return (
    <Section id="deployment-profile" title={m.admin.profileTitle} hint={m.admin.profileHint}>
      {profile.error !== null && <ReadFailed error={profile.error} onRetry={profile.reload} />}
      {profile.data !== undefined && (
        <>
          <div className="flex flex-col gap-2">
            <h3 className={labelClass}>{m.admin.deploymentIcon}</h3>
            <p className={hintClass}>{m.admin.deploymentIconHint}</p>
            <div className="flex flex-wrap items-center gap-3">
              {icon !== null && (
                <img
                  src={icon.downloadUrl}
                  alt=""
                  width={96}
                  height={96}
                  className="aspect-square rounded-full object-cover"
                />
              )}
              <IconPicker
                onIcon={async (iconId) => {
                  setIconError(null);
                  await update({ icon: iconId }, setIconError);
                }}
              >
                {(open, uploading) => (
                  <Button onPress={open} isDisabled={uploading} className={secondaryButtonClass}>
                    {icon === null ? m.admin.addDeploymentIcon : m.admin.changeDeploymentIcon}
                  </Button>
                )}
              </IconPicker>
              {icon !== null && (
                <Button
                  onPress={() => {
                    setIconError(null);
                    void update({ icon: null }, setIconError);
                  }}
                  className={secondaryButtonClass}
                >
                  {m.admin.removeDeploymentIcon}
                </Button>
              )}
            </div>
            {iconError !== null && (
              <p role="alert" className={alertClass}>
                {iconError}
              </p>
            )}
          </div>
          <DisplayNameForm saved={profile.data.displayName ?? ""} update={update} />
        </>
      )}
    </Section>
  );
}

/**
 * The display name's field, starting from the name the server has; an empty name is saved as
 * none.
 */
function DisplayNameForm({
  saved,
  update,
}: {
  /** The name as the server has it, empty for none. */
  saved: string;
  update: (change: DeploymentProfileUpdateRequest, fail: (e: string) => void) => Promise<boolean>;
}) {
  const m = useMessages();
  const [name, setName] = useState(saved);
  const [saving, setSaving] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        const trimmed = name.trim();
        setSaving(true);
        setDone(false);
        setError(null);
        void update({ displayName: trimmed === "" ? null : trimmed }, setError)
          .then(setDone)
          .finally(() => {
            setSaving(false);
          });
      }}
      className="flex flex-col gap-3"
    >
      <TextField
        value={name}
        onChange={(value) => {
          setName(value);
          setDone(false);
        }}
        maxLength={DISPLAY_NAME_MAX_CHARS}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.admin.displayName}</Label>
        <Input className={inputClass} />
        <Text slot="description" className={hintClass}>
          {m.admin.displayNameHint}
        </Text>
      </TextField>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className="flex items-center gap-3">
        <Button
          type="submit"
          isDisabled={saving || name.trim() === saved}
          className={primaryButtonClass}
        >
          {saving ? m.admin.savingDisplayName : m.admin.saveDisplayName}
        </Button>
        {done && (
          <span role="status" className="text-sm text-ink-muted">
            {m.admin.displayNameSaved}
          </span>
        )}
      </div>
    </Form>
  );
}
