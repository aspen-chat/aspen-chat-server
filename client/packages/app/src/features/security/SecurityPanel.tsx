import {
  ApiProblemError,
  PasskeyCancelledError,
  type Passkey,
  type PasskeyTransport,
} from "@aspen/protocol";
import { KeyIcon, PencilSimpleIcon, TrashIcon } from "@phosphor-icons/react";
import { useCallback, useState } from "react";
import { Button, FieldError, Form, Input, Label, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import * as api from "./api";
import { QrCode } from "./QrCode";
import { RecoveryCodesDialog } from "./RecoveryCodesDialog";
import { ReauthProvider } from "./reauth";
import { useReauth } from "./reauthContext";
import { useSecuritySettings } from "./useSecuritySettings";

function errorText(e: unknown): string | null {
  if (e instanceof PasskeyCancelledError) {
    return null;
  }
  return e instanceof ApiProblemError ? e.message : String(e);
}

const DATE: Intl.DateTimeFormatOptions = { dateStyle: "medium" };

/**
 * Authenticator app, passkeys, and recovery codes. Changes that need a fresh verification ask
 * for one first.
 */
export function SecurityPanel({ transport }: { transport: PasskeyTransport | null }) {
  const m = useMessages();
  const { settings, failed, reload } = useSecuritySettings();
  if (settings === null) {
    return <p className={hintClass}>{failed ? m.security.loadFailed : m.security.loading}</p>;
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
    <div className="flex flex-col gap-5">
      <p className="text-sm">
        {settings.twoFactorEnabled ? m.security.twoFactorOn : m.security.twoFactorOff}
        {settings.twoFactorRequired && ` ${m.security.twoFactorRequired}`}
      </p>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <AuthenticatorSection settings={settings} change={change} />
      <PasskeySection settings={settings} transport={transport} change={change} />
      <RecoverySection settings={settings} change={change} />
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
    <section aria-labelledby="security-authenticator" className="flex flex-col gap-2">
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
    <section aria-labelledby="security-passkeys" className="flex flex-col gap-2">
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

function RecoverySection({ settings, change }: { settings: api.SecuritySettings; change: Change }) {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  return (
    <section aria-labelledby="security-recovery" className="flex flex-col gap-2">
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
