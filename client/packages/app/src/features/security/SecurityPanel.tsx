import {
  ApiProblemError,
  PasskeyCancelledError,
  type Passkey,
  type PasskeyTransport,
} from "@aspen/protocol";
import { KeyIcon, PencilSimpleIcon, TrashIcon } from "@phosphor-icons/react";
import { useCallback, useState } from "react";
import { Button, FieldError, Form, Input, Label, Text, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { useMe } from "@/api/hooks";
import { PASSWORD_MIN_LENGTH } from "@/features/auth/password";
import { OtherDeviceSignIn } from "@/features/auth/OtherDeviceSignIn";
import { detectShell } from "@/config";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { planeClass, secondaryButtonClass } from "@/features/invites/dialog";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import * as api from "./api";
import { QrCode } from "@/features/qr/QrCode";
import { RecoveryCodesDialog } from "./RecoveryCodesDialog";
import { ReauthProvider } from "./reauth";
import { useReauth } from "./reauthContext";
import { useSecuritySettings } from "./useSecuritySettings";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";

function errorText(e: unknown): string | null {
  if (e instanceof PasskeyCancelledError) {
    return null;
  }
  return e instanceof ApiProblemError ? e.message : String(e);
}

const DATE: Intl.DateTimeFormatOptions = { dateStyle: "medium" };

/**
 * The password, authenticator app, passkeys, recovery codes, signing in other devices by a QR
 * code, and signing out everywhere else. Changes that need a fresh verification ask for one first.
 */
export function SecurityPanel({ transport }: { transport: PasskeyTransport | null }) {
  const m = useMessages();
  const { settings, failed, reload } = useSecuritySettings();
  if (settings === null) {
    if (failed) {
      return <p className={hintClass}>{m.security.loadFailed}</p>;
    }
    return (
      <div aria-busy="true" className="flex flex-col gap-3">
        <LoadingLabel text={m.security.loading} />
        {["h-28", "h-20", "h-24"].map((height) => (
          <Skeleton key={height} className={height + " w-full rounded-lg"} />
        ))}
      </div>
    );
  }
  return (
    <ReauthProvider settings={settings} transport={transport}>
      <Sections settings={settings} transport={transport} reload={reload} />
    </ReauthProvider>
  );
}

function Sections({
  settings,
  transport,
  reload,
}: {
  settings: api.SecuritySettings;
  transport: PasskeyTransport | null;
  reload: () => Promise<void>;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const me = useMe();
  const [codes, setCodes] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** Runs a change, shows any new recovery codes, and reloads the settings. */
  const change = useCallback(
    async (run: () => Promise<string[] | null | undefined>) => {
      setError(null);
      try {
        const issued = await run();
        if (issued != null) {
          setCodes(issued);
        }
      } catch (e) {
        setError(errorText(e));
      }
      await reload();
    },
    [reload],
  );
  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm">
        {settings.twoFactorEnabled ? m.security.twoFactorOn : m.security.twoFactorOff}
        {settings.twoFactorRequired && ` ${m.security.twoFactorRequired}`}
      </p>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {/* A user of another deployment has no password here: their home holds it. */}
      {me?.homeDomain == null && <PasswordSection />}
      <AuthenticatorSection settings={settings} change={change} />
      <PasskeySection settings={settings} transport={transport} change={change} />
      <RecoverySection settings={settings} change={change} />
      {/* A user of another deployment signs devices in at home; an account still owing a
          second factor has no sign-in to give. */}
      {me?.homeDomain == null && client.session?.twoFactorEnrollmentRequired !== true && (
        <OtherDevicesSection />
      )}
      {/* A user of another deployment's sign-ins are their home's to end, and an account still
          owing a second factor may do nothing else first. */}
      {me?.homeDomain == null && client.session?.twoFactorEnrollmentRequired !== true && (
        <SignOutEverywhereSection />
      )}
      <RecoveryCodesDialog
        codes={codes}
        onClose={() => {
          setCodes(null);
          // Codes come with the first second factor; once they are saved, a server's
          // requirement to add one is met.
          client.markEnrolled();
        }}
      />
    </div>
  );
}

type Change = (run: () => Promise<string[] | null | undefined>) => Promise<void>;

function SectionHeading({ id, children }: { id: string; children: string }) {
  return (
    <h3 id={id} className="text-sm font-semibold text-ink-muted">
      {children}
    </h3>
  );
}

/**
 * Changing the password: the current one, then the new one twice, which the browser's password
 * manager can fill and save. Wrong or too short, the field at fault says so; changed, the
 * section says where the user is still signed in.
 */
function PasswordSection() {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  const me = useMe();
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [changed, setChanged] = useState(false);
  const [errors, setErrors] = useState<{
    current?: string | undefined;
    password?: string | undefined;
    confirm?: string | undefined;
    form?: string | undefined;
  }>({});

  // A field marked invalid blocks the form's next submission, so its error goes as soon as the
  // field is edited.
  const clearError = (field: "current" | "password" | "confirm") => {
    setErrors((shown) => (shown[field] === undefined ? shown : { ...shown, [field]: undefined }));
  };

  async function submit(form: HTMLFormElement) {
    const data = new FormData(form);
    const current = formString(data, "currentPassword");
    const password = formString(data, "newPassword");
    const confirm = formString(data, "confirmPassword");
    setErrors({});
    if (password.length < PASSWORD_MIN_LENGTH) {
      setErrors({ password: m.passwordHint });
      return;
    }
    if (password !== confirm) {
      setErrors({ confirm: m.passwordsDoNotMatch });
      return;
    }
    setPending(true);
    try {
      // `withReauth` answers `undefined` when the user declines to verify, so the change
      // answers `true` to tell its success apart.
      const done = await withReauth(async () => {
        await api.changePassword(client, current, password);
        return true;
      });
      if (done === true) {
        setOpen(false);
        setChanged(true);
      }
    } catch (e) {
      if (e instanceof ApiProblemError && e.code === "oldPasswordIncorrect") {
        setErrors({ current: e.message });
      } else if (e instanceof ApiProblemError && e.code === "passwordRequirementsNotMet") {
        setErrors({ password: e.message });
      } else {
        const text = errorText(e);
        setErrors(text === null ? {} : { form: text });
      }
    } finally {
      setPending(false);
    }
  }

  return (
    <section aria-labelledby="security-password" className={planeClass}>
      <SectionHeading id="security-password">{m.security.passwordHeading}</SectionHeading>
      {!open ? (
        <div className="flex items-center justify-between gap-2">
          <p className="text-sm">
            {changed ? m.security.passwordChanged : m.security.passwordHint}
          </p>
          <Button
            onPress={() => {
              setErrors({});
              setChanged(false);
              setOpen(true);
            }}
            className={secondaryButtonClass + " shrink-0"}
          >
            {m.security.changePassword}
          </Button>
        </div>
      ) : (
        <Form
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            void submit(event.currentTarget);
          }}
        >
          {/* Tells a password manager whose password this is, so it updates the right entry. */}
          <input
            type="text"
            name="username"
            autoComplete="username"
            value={me?.name ?? ""}
            readOnly
            hidden
          />
          <TextField
            name="currentPassword"
            type="password"
            onChange={() => {
              clearError("current");
            }}
            isRequired
            autoFocus
            autoComplete="current-password"
            isInvalid={errors.current !== undefined}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.security.currentPassword}</Label>
            <Input className={inputClass} />
            <FieldError className="text-sm text-danger">{errors.current}</FieldError>
          </TextField>
          <TextField
            name="newPassword"
            type="password"
            onChange={() => {
              clearError("password");
            }}
            isRequired
            autoComplete="new-password"
            isInvalid={errors.password !== undefined}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.security.newPassword}</Label>
            <Input className={inputClass} />
            {errors.password === undefined ? (
              <Text slot="description" className={hintClass}>
                {m.passwordHint}
              </Text>
            ) : (
              <FieldError className="text-sm text-danger">{errors.password}</FieldError>
            )}
          </TextField>
          <TextField
            name="confirmPassword"
            type="password"
            onChange={() => {
              clearError("confirm");
            }}
            isRequired
            autoComplete="new-password"
            isInvalid={errors.confirm !== undefined}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.security.confirmNewPassword}</Label>
            <Input className={inputClass} />
            <FieldError className="text-sm text-danger">{errors.confirm}</FieldError>
          </TextField>
          {errors.form !== undefined && (
            <p role="alert" className={alertClass}>
              {errors.form}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button
              onPress={() => {
                setOpen(false);
              }}
              className={secondaryButtonClass}
            >
              {m.security.cancel}
            </Button>
            <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
              {pending ? m.security.changingPassword : m.security.changePassword}
            </Button>
          </div>
        </Form>
      )}
    </section>
  );
}

function AuthenticatorSection({
  settings,
  change,
}: {
  settings: api.SecuritySettings;
  change: Change;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  const [enrollment, setEnrollment] = useState<api.TotpEnrollment | null>(null);
  const [confirmError, setConfirmError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function begin() {
    await change(async () => {
      const started = await withReauth(() => api.beginTotp(client));
      if (started !== undefined) {
        setConfirmError(null);
        setEnrollment(started);
      }
      return null;
    });
  }

  async function confirm(code: string) {
    setPending(true);
    setConfirmError(null);
    try {
      const issued = await withReauth(() => api.confirmTotp(client, code));
      setEnrollment(null);
      await change(() => Promise.resolve(issued));
    } catch (e) {
      setConfirmError(errorText(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <section aria-labelledby="security-authenticator" className={planeClass}>
      <SectionHeading id="security-authenticator">{m.security.authenticatorHeading}</SectionHeading>
      {enrollment === null ? (
        <div className="flex items-center justify-between gap-2">
          <p className="text-sm">
            {settings.totp ? m.security.authenticatorOn : m.security.authenticatorOff}
          </p>
          {settings.totp ? (
            <Button
              onPress={() => {
                void change(async () => {
                  await withReauth(() => api.removeTotp(client));
                  return null;
                });
              }}
              className={secondaryButtonClass + " text-danger"}
            >
              {m.security.remove}
            </Button>
          ) : (
            <Button
              onPress={() => {
                void begin();
              }}
              className={secondaryButtonClass}
            >
              {m.security.setUp}
            </Button>
          )}
        </div>
      ) : (
        <Form
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            void confirm(formString(new FormData(event.currentTarget), "code"));
          }}
        >
          <p className={hintClass}>{m.security.scanPrompt}</p>
          <div className="flex flex-wrap items-center gap-4">
            <QrCode text={enrollment.uri} label={m.security.qrLabel} />
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <span className={labelClass}>{m.security.secretLabel}</span>
              <code className="break-all rounded bg-surface px-2 py-1 text-sm select-all">
                {enrollment.secret.replace(/(.{4})/g, "$1 ").trim()}
              </code>
            </div>
          </div>
          <TextField
            name="code"
            isRequired
            autoFocus
            autoComplete="one-time-code"
            inputMode="numeric"
            className={fieldClass}
          >
            <Label className={labelClass}>{m.security.confirmCodeLabel}</Label>
            <Input className={inputClass + " font-mono tracking-widest"} />
            <FieldError className="text-sm text-danger" />
          </TextField>
          {confirmError !== null && (
            <p role="alert" className={alertClass}>
              {confirmError}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button
              onPress={() => {
                setEnrollment(null);
              }}
              className={secondaryButtonClass}
            >
              {m.security.cancel}
            </Button>
            <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
              {pending ? m.twoFactor.verifying : m.security.turnOn}
            </Button>
          </div>
        </Form>
      )}
    </section>
  );
}

function PasskeySection({
  settings,
  transport,
  change,
}: {
  settings: api.SecuritySettings;
  transport: PasskeyTransport | null;
  change: Change;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  const [pending, setPending] = useState(false);
  return (
    <section aria-labelledby="security-passkeys" className={planeClass}>
      <SectionHeading id="security-passkeys">{m.security.passkeysHeading}</SectionHeading>
      <p className={hintClass}>{m.security.passkeysHint}</p>
      {settings.passkeys.length === 0 ? (
        <p className="text-sm">{m.security.noPasskeys}</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {settings.passkeys.map((passkey) => (
            <PasskeyRow key={passkey.id} passkey={passkey} change={change} />
          ))}
        </ul>
      )}
      {transport === null ? (
        <p className={hintClass}>{m.security.passkeysUnavailable}</p>
      ) : (
        <Form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            const form = event.currentTarget;
            const name = formString(new FormData(form), "name").trim();
            setPending(true);
            void change(async () => {
              const added = await withReauth(() =>
                client.runPasskeyCeremony(
                  { purpose: "register", ...(name.length > 0 ? { name } : {}) },
                  transport,
                ),
              );
              form.reset();
              return added?.outcome === "passkeyAdded" ? added.recoveryCodes : null;
            }).finally(() => {
              setPending(false);
            });
          }}
        >
          {/* The name shrinks to leave the button room, down to a width that still reads, and
              the button wraps below it on a screen narrower than that. */}
          <TextField name="name" maxLength={64} className={fieldClass + " min-w-40 flex-1"}>
            <Label className={labelClass}>{m.security.passkeyNameLabel}</Label>
            <Input
              className={inputClass + " w-full min-w-0"}
              placeholder={m.security.passkeyNamePlaceholder}
            />
          </TextField>
          <Button
            type="submit"
            isDisabled={pending}
            className={primaryButtonClass + " flex shrink-0 items-center gap-1.5 whitespace-nowrap"}
          >
            <KeyIcon size={16} aria-hidden="true" />
            {pending
              ? transport.kind === "handoff"
                ? m.passkeyWaitingBrowser
                : m.passkeyWaiting
              : m.security.addPasskey}
          </Button>
        </Form>
      )}
    </section>
  );
}

function PasskeyRow({ passkey, change }: { passkey: Passkey; change: Change }) {
  const dateFormat = useDateFormat(DATE);
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  const [renaming, setRenaming] = useState(false);
  const added = format(m.security.passkeyAdded, {
    date: dateFormat.format(new Date(passkey.createdAt)),
  });
  const used =
    passkey.lastUsedAt == null
      ? null
      : format(m.security.passkeyLastUsed, {
          date: dateFormat.format(new Date(passkey.lastUsedAt)),
        });
  if (renaming) {
    return (
      <li>
        <Form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            const name = formString(new FormData(event.currentTarget), "name");
            setRenaming(false);
            void change(async () => {
              await api.renamePasskey(client, passkey.id, name);
              return null;
            });
          }}
        >
          <TextField
            name="name"
            defaultValue={passkey.name}
            maxLength={64}
            autoFocus
            aria-label={m.security.passkeyNameLabel}
            className={fieldClass + " flex-1"}
          >
            <Input className={inputClass} />
          </TextField>
          <Button type="submit" className={secondaryButtonClass}>
            {m.security.save}
          </Button>
          <Button
            onPress={() => {
              setRenaming(false);
            }}
            className={secondaryButtonClass}
          >
            {m.security.cancel}
          </Button>
        </Form>
      </li>
    );
  }
  return (
    <li className="flex items-center justify-between gap-2 rounded-md bg-surface px-3 py-2">
      <div className="min-w-0">
        <p className="truncate text-sm font-medium">{passkey.name}</p>
        <p className="text-xs text-ink-muted">{used === null ? added : `${added}, ${used}`}</p>
      </div>
      <div className="flex shrink-0 gap-1">
        <Button
          aria-label={`${m.security.rename} ${passkey.name}`}
          onPress={() => {
            setRenaming(true);
          }}
          className={secondaryButtonClass}
        >
          <PencilSimpleIcon size={14} aria-hidden="true" />
        </Button>
        <Button
          aria-label={`${m.security.remove} ${passkey.name}`}
          onPress={() => {
            void change(async () => {
              await withReauth(() => api.removePasskey(client, passkey.id));
              return null;
            });
          }}
          className={secondaryButtonClass + " text-danger"}
        >
          <TrashIcon size={14} aria-hidden="true" />
        </Button>
      </div>
    </li>
  );
}

/** Signing a phone in from this computer, or a computer in from this phone, by a QR code. */
function OtherDevicesSection() {
  const m = useMessages();
  return (
    <section aria-labelledby="security-devices" className={planeClass}>
      <SectionHeading id="security-devices">{m.deviceLink.heading}</SectionHeading>
      <p className={hintClass}>
        {detectShell() === "mobile" ? m.deviceLink.sectionHintPhone : m.deviceLink.sectionHint}
      </p>
      <OtherDeviceSignIn
        className={secondaryButtonClass + " flex items-center gap-1.5 self-start"}
      />
    </section>
  );
}

/** Ending every other sign-in of the account, for someone who fears another device has it. */
function SignOutEverywhereSection() {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  const [pending, setPending] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <section aria-labelledby="security-sign-ins" className={planeClass}>
      <SectionHeading id="security-sign-ins">{m.security.signInsHeading}</SectionHeading>
      <div className="flex items-center justify-between gap-2">
        <p className="text-sm">{done ? m.security.signedOutElsewhere : m.security.signInsHint}</p>
        <Button
          isDisabled={pending}
          onPress={() => {
            setPending(true);
            setError(null);
            // `withReauth` answers `undefined` when the user declines to verify.
            withReauth(async () => {
              await api.endOtherSignIns(client);
              return true;
            })
              .then((ended) => {
                if (ended === true) {
                  setDone(true);
                }
              })
              .catch((e: unknown) => {
                setError(errorText(e));
              })
              .finally(() => {
                setPending(false);
              });
          }}
          className={secondaryButtonClass + " shrink-0 text-danger"}
        >
          {m.security.signOutEverywhere}
        </Button>
      </div>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
    </section>
  );
}

function RecoverySection({ settings, change }: { settings: api.SecuritySettings; change: Change }) {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  return (
    <section aria-labelledby="security-recovery" className={planeClass}>
      <SectionHeading id="security-recovery">{m.security.recoveryHeading}</SectionHeading>
      {settings.twoFactorEnabled ? (
        <div className="flex items-center justify-between gap-2">
          <p className="text-sm">
            {format(m.security.recoveryRemaining, {
              count: String(settings.recoveryCodesRemaining),
            })}
          </p>
          <Button
            onPress={() => {
              void change(() => withReauth(() => api.regenerateRecoveryCodes(client)));
            }}
            className={secondaryButtonClass}
          >
            {m.security.regenerate}
          </Button>
        </div>
      ) : (
        <p className={hintClass}>{m.security.recoveryOff}</p>
      )}
    </section>
  );
}
