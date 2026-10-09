import { CaretDownIcon, EnvelopeSimpleIcon } from "@phosphor-icons/react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Button,
  ComboBox,
  FieldError,
  Form,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
  Text,
  TextField,
} from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { problemText } from "@/api/problemText";
import { usePasskeyTransport } from "@/features/auth/passkeyTransport";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  linkButtonClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  optionClass,
  planeClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { ReauthProvider } from "@/features/security/reauth";
import { useReauth } from "@/features/security/reauthContext";
import { useSecuritySettings } from "@/features/security/useSecuritySettings";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import * as api from "./api";
import { useEmailPolicy } from "./policy";
import { deviceTimeZone, timeZones } from "./timeZones";

/**
 * The account's email address and what it receives there: the address, which can be added,
 * changed, or removed (each a security change, so it may ask the user to confirm it's them
 * first), its verification by the code mailed to it, and, outside the verification gate,
 * whether the profile shows it, the newsletter, and the daily digest. `gate` is the screen an
 * account owing a verified address sees, which offers only what verifies it; verifying lifts
 * the gate. `changes` counts `emailAccountChanged` events, on each of which the account is read
 * again.
 */
export function EmailPanel({ gate = false, changes = 0 }: { gate?: boolean; changes?: number }) {
  const m = useMessages();
  const client = useAspenClient();
  const transport = usePasskeyTransport();
  const security = useSecuritySettings();
  const policy = useEmailPolicy();
  const [account, setAccount] = useState<api.EmailAccount | null>(null);
  const [failed, setFailed] = useState(false);

  const reload = useCallback(async () => {
    try {
      setAccount(await api.fetchEmail(client));
      setFailed(false);
    } catch {
      setFailed(true);
    }
  }, [client]);
  // Read on opening and again on each change another device or the server announces.
  useEffect(() => {
    let live = true;
    api.fetchEmail(client).then(
      (read) => {
        if (live) {
          setAccount(read);
          setFailed(false);
        }
      },
      () => {
        if (live) {
          setFailed(true);
        }
      },
    );
    return () => {
      live = false;
    };
  }, [client, changes]);

  if (account === null || security.settings === null || policy === null) {
    if (failed || security.failed) {
      return <p className={hintClass}>{m.email.loadFailed}</p>;
    }
    return (
      <div aria-busy="true" className="flex flex-col gap-3">
        <LoadingLabel text={m.email.loading} />
        <Skeleton className="h-24 w-full rounded-lg" />
      </div>
    );
  }
  const adopt = (next: api.EmailAccount) => {
    setAccount(next);
    // Verified, or with nothing left to verify, the account no longer owes the gate.
    if (next.verified || next.address == null) {
      client.markEmailVerified();
    }
  };
  return (
    <ReauthProvider settings={security.settings} transport={transport}>
      <div className="flex flex-col gap-4">
        <AddressSection
          account={account}
          gate={gate}
          required={policy.required}
          onChange={adopt}
          onRemoved={() => {
            void reload().then(() => {
              client.markEmailVerified();
            });
          }}
        />
        {account.address != null && !account.verified && (
          <VerifySection address={account.address} onVerified={adopt} />
        )}
        {!gate && account.address != null && (
          <PreferencesSection account={account} newsletter={policy.newsletter} onChange={adopt} />
        )}
      </div>
    </ReauthProvider>
  );
}

/** The address, and adding, changing, or removing it. */
function AddressSection({
  account,
  gate,
  required,
  onChange,
  onRemoved,
}: {
  account: api.EmailAccount;
  gate: boolean;
  required: boolean;
  onChange: (account: api.EmailAccount) => void;
  onRemoved: () => void;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  const [editing, setEditing] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const address = account.address ?? null;

  async function save(form: HTMLFormElement) {
    const given = formString(new FormData(form), "address").trim();
    setPending(true);
    setFieldError(null);
    setError(null);
    try {
      const changed = await withReauth(() => api.setAddress(client, given));
      if (changed !== undefined) {
        onChange(changed);
        setEditing(false);
      }
    } catch (e) {
      setFieldError(problemText(e));
    } finally {
      setPending(false);
    }
  }

  async function remove() {
    setPending(true);
    setError(null);
    try {
      const done = await withReauth(async () => {
        await api.removeAddress(client);
        return true;
      });
      if (done === true) {
        onRemoved();
      }
    } catch (e) {
      setError(problemText(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <section aria-labelledby="email-address" className={planeClass}>
      <h3 id="email-address" className="text-sm font-semibold text-ink-muted">
        {m.email.addressHeading}
      </h3>
      {address === null ? (
        <p className={hintClass}>{m.email.noAddress}</p>
      ) : (
        <p className="flex flex-wrap items-center gap-2 text-sm">
          <EnvelopeSimpleIcon size={16} aria-hidden="true" className="shrink-0 text-ink-muted" />
          <span className="font-medium break-all">{address}</span>
          <span className={account.verified ? "text-ink-muted" : "text-danger"}>
            {account.verified ? m.email.verified : m.email.unverified}
          </span>
        </p>
      )}
      {address !== null && !gate && <p className={hintClass}>{m.email.addressHint}</p>}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {editing ? (
        <Form
          onSubmit={(event) => {
            event.preventDefault();
            void save(event.currentTarget);
          }}
          className="flex flex-col gap-3"
        >
          <TextField
            name="address"
            type="email"
            isRequired
            autoFocus
            autoComplete="email"
            // A field marked invalid blocks the form's next submission, so its error goes as
            // soon as the field is edited.
            onChange={() => {
              setFieldError(null);
            }}
            isInvalid={fieldError !== null}
            className={fieldClass}
          >
            <Label className={labelClass}>
              {address === null ? m.email.addressLabel : m.email.newAddressLabel}
            </Label>
            <Input className={inputClass} spellCheck={false} autoCapitalize="off" />
            <Text slot="description" className={hintClass}>
              {m.email.codeWillBeSent}
            </Text>
            <FieldError className="text-sm text-danger">{fieldError}</FieldError>
          </TextField>
          <div className="flex flex-wrap gap-2">
            <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
              {pending ? m.email.saving : m.email.saveAddress}
            </Button>
            <Button
              onPress={() => {
                setEditing(false);
                setFieldError(null);
              }}
              className={secondaryButtonClass}
            >
              {m.email.cancel}
            </Button>
          </div>
        </Form>
      ) : (
        <div className="flex flex-wrap gap-2">
          <Button
            onPress={() => {
              setEditing(true);
            }}
            className={secondaryButtonClass}
          >
            {address === null ? m.email.addAddress : m.email.changeAddress}
          </Button>
          {address !== null && !gate && !required && (
            <Button
              isDisabled={pending}
              onPress={() => {
                void remove();
              }}
              className={secondaryButtonClass + " text-danger"}
            >
              {m.email.removeAddress}
            </Button>
          )}
        </div>
      )}
    </section>
  );
}

/** Typing the code mailed to the address, or asking for another. */
function VerifySection({
  address,
  onVerified,
}: {
  address: string;
  onVerified: (account: api.EmailAccount) => void;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const [pending, setPending] = useState(false);
  const [codeError, setCodeError] = useState<string | null>(null);
  const [resent, setResent] = useState<"idle" | "sending" | "sent">("idle");
  const [resendError, setResendError] = useState<string | null>(null);

  async function submit(form: HTMLFormElement) {
    const code = formString(new FormData(form), "code").trim();
    setPending(true);
    setCodeError(null);
    try {
      onVerified(await api.verify(client, code));
    } catch (e) {
      setCodeError(problemText(e));
    } finally {
      setPending(false);
    }
  }

  async function resend() {
    setResent("sending");
    setResendError(null);
    try {
      await api.resendCode(client);
      setResent("sent");
    } catch (e) {
      setResendError(problemText(e));
      setResent("idle");
    }
  }

  return (
    <section aria-labelledby="email-verify" className={planeClass}>
      <h3 id="email-verify" className="text-sm font-semibold text-ink-muted">
        {m.email.verifyHeading}
      </h3>
      <p className={hintClass}>{format(m.email.verifyPrompt, { address })}</p>
      <Form
        onSubmit={(event) => {
          event.preventDefault();
          void submit(event.currentTarget);
        }}
        className="flex flex-col gap-3"
      >
        <TextField
          name="code"
          isRequired
          inputMode="numeric"
          autoComplete="one-time-code"
          maxLength={6}
          // A field marked invalid blocks the form's next submission, so its error goes as soon
          // as the field is edited.
          onChange={() => {
            setCodeError(null);
          }}
          isInvalid={codeError !== null}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.email.codeLabel}</Label>
          <Input className={inputClass + " font-mono tracking-widest sm:max-w-40"} />
          <FieldError className="text-sm text-danger">{codeError}</FieldError>
        </TextField>
        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
            {pending ? m.email.verifying : m.email.verify}
          </Button>
          <Button
            isDisabled={resent === "sending"}
            onPress={() => {
              void resend();
            }}
            className={linkButtonClass + " text-sm"}
          >
            {m.email.resend}
          </Button>
          {resent === "sent" && (
            <span role="status" className={hintClass}>
              {m.email.resent}
            </span>
          )}
        </div>
        {resendError !== null && (
          <p role="alert" className={alertClass}>
            {resendError}
          </p>
        )}
      </Form>
    </section>
  );
}

/** Whether the profile shows the address, the newsletter, and the daily digest. */
function PreferencesSection({
  account,
  newsletter,
  onChange,
}: {
  account: api.EmailAccount;
  newsletter: boolean;
  onChange: (account: api.EmailAccount) => void;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const hour = useDateFormat({ hour: "numeric" });

  async function change(update: api.EmailAccountUpdate) {
    setPending(true);
    setError(null);
    try {
      onChange(await api.updateEmail(client, update));
    } catch (e) {
      setError(problemText(e));
    } finally {
      setPending(false);
    }
  }

  const hours = Array.from({ length: 24 }, (_, h) => ({
    id: String(h),
    label: hour.format(new Date(2000, 0, 1, h)),
  }));

  return (
    <section aria-labelledby="email-preferences" className={planeClass}>
      <h3 id="email-preferences" className="text-sm font-semibold text-ink-muted">
        {m.email.preferencesHeading}
      </h3>
      {!account.verified && <p className={hintClass}>{m.email.preferencesUnverified}</p>}
      <ChoiceCheckbox
        isSelected={account.shown}
        isDisabled={pending}
        onChange={(shown) => {
          void change({ shown });
        }}
        label={m.email.showOnProfile}
        hint={m.email.showOnProfileHint}
      />
      {(newsletter || account.newsletter) && (
        <ChoiceCheckbox
          isSelected={account.newsletter}
          isDisabled={pending}
          onChange={(subscribed) => {
            void change({ newsletter: subscribed });
          }}
          label={m.email.newsletter}
          hint={m.email.newsletterHint}
        />
      )}
      <ChoiceCheckbox
        isSelected={account.digest}
        isDisabled={pending}
        onChange={(digest) => {
          // A digest first turned on is sent by this device's clock, unless one was chosen.
          const zone = deviceTimeZone();
          void change(
            digest && account.digestTimeZone === "UTC" && zone !== "UTC"
              ? { digest, digestTimeZone: zone }
              : { digest },
          );
        }}
        label={m.email.digest}
        hint={m.email.digestHint}
      />
      {account.digest && (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <ZonePicker
            // Chosen anew, the field starts from the zone the server now has.
            key={account.digestTimeZone}
            zone={account.digestTimeZone}
            isDisabled={pending}
            onChange={(zone) => {
              void change({ digestTimeZone: zone });
            }}
          />
          <Select
            value={String(account.digestHour)}
            onChange={(key) => {
              if (typeof key === "string") {
                void change({ digestHour: Number(key) });
              }
            }}
            isDisabled={pending}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.email.digestHour}</Label>
            <Button className={selectButtonClass}>
              <SelectValue className="truncate" />
              <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
            </Button>
            <Popover className={selectPopoverClass + " max-h-72 overflow-y-auto"}>
              <ListBox items={hours}>
                {(item) => (
                  <ListBoxItem id={item.id} textValue={item.label} className={optionClass}>
                    {item.label}
                  </ListBoxItem>
                )}
              </ListBox>
            </Popover>
          </Select>
        </div>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
    </section>
  );
}

/** The digest's time zone, found by typing part of its name. */
function ZonePicker({
  zone,
  isDisabled,
  onChange,
}: {
  zone: string;
  isDisabled: boolean;
  onChange: (zone: string) => void;
}) {
  const m = useMessages();
  const zones = useMemo(() => timeZones(zone), [zone]);
  const [query, setQuery] = useState(zone);
  // While the field shows the chosen zone, the list is not narrowed by it.
  const narrowing = query === zone ? "" : query.toLowerCase();
  const shown = zones.filter((z) => z.toLowerCase().includes(narrowing)).map((id) => ({ id }));
  return (
    <ComboBox
      items={shown}
      inputValue={query}
      onInputChange={setQuery}
      value={zone}
      onChange={(key) => {
        if (typeof key === "string" && key !== zone) {
          onChange(key);
        }
      }}
      onBlur={() => {
        setQuery(zone);
      }}
      menuTrigger="focus"
      isDisabled={isDisabled}
      className={fieldClass}
    >
      <Label className={labelClass}>{m.email.timeZone}</Label>
      <div className="relative">
        <Input className={inputClass + " w-full pe-9"} />
        <Button className="absolute top-1/2 end-2 -translate-y-1/2 rounded p-1 text-ink-muted outline-none">
          <CaretDownIcon size={14} aria-hidden="true" />
        </Button>
      </div>
      <Popover className={selectPopoverClass + " max-h-72 overflow-y-auto"}>
        <ListBox className="outline-none">
          {(item: { id: string }) => (
            <ListBoxItem id={item.id} textValue={item.id} className={optionClass}>
              {item.id}
            </ListBoxItem>
          )}
        </ListBox>
      </Popover>
    </ComboBox>
  );
}
