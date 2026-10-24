import type { IdentityProviderSettings } from "@aspen/protocol";
import { CopyIcon } from "@phosphor-icons/react";
import { useCallback, useRef, useState } from "react";
import { Button, FieldError, Form, Input, Label, Text, TextField } from "react-aria-components";
import { problemText } from "@/api/problemText";
import { useSync } from "@/api/hooks";
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
import { ChoiceCheckbox } from "@/features/layout/choices";
import { copyText } from "@/features/layout/clipboard";
import { ConfirmModal } from "@/features/layout/ConfirmModal";
import { Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import {
  createRequest,
  draftOf,
  NEW_PROVIDER,
  providerPatch,
  type Draft,
} from "./identityProviderDrafts";

/** The most linked accounts the server counts; it answers one more for "more than this". */
const LINKED_COUNTED = 1000;

const DATE: Intl.DateTimeFormatOptions = { dateStyle: "medium" };

/**
 * The deployment's OpenID Connect identity providers, for holders of Manage identity
 * providers: the two addresses to register at each, the providers with how many accounts are
 * linked to each, and adding, changing, and removing them. Adding one, or changing its issuer or
 * client, waits while the server reads the provider's discovery document, and says what went
 * wrong when it cannot. The client secret is write-only: an edit may replace it, never show it.
 */
export function IdentityProvidersSection() {
  const m = useMessages();
  const a = m.identityProviders.admin;
  const sync = useSync();
  const load = useCallback(() => sync.admin.identityProviders(), [sync]);
  const read = useAdminRead(load);
  const list = read.data;
  return (
    <>
      <Section id="admin-identity-providers" title={a.title} hint={a.hint}>
        {read.error !== null && <ReadFailed error={read.error} onRetry={read.reload} />}
        {list === undefined ? (
          read.error === null && (
            <div aria-busy="true" className="flex flex-col gap-2">
              <Skeleton className="h-14 w-full" />
              <Skeleton className="h-14 w-full" />
            </div>
          )
        ) : (
          <>
            {list.providers.length === 0 ? (
              <p className="text-sm text-ink-muted">{a.none}</p>
            ) : (
              <ul className="flex flex-col divide-y divide-line rounded-md border border-line">
                {list.providers.map((provider) => (
                  <li key={provider.id}>
                    <ProviderRow provider={provider} onChanged={read.reload} />
                  </li>
                ))}
              </ul>
            )}
            {list.providers.length >= list.maxProviders ? (
              <p className={hintClass}>{format(a.atMost, { max: list.maxProviders })}</p>
            ) : (
              <AddProvider onAdded={read.reload} />
            )}
          </>
        )}
      </Section>
      {list !== undefined && (
        <Section
          id="admin-identity-provider-addresses"
          title={a.registerTitle}
          hint={a.registerHint}
        >
          <Address label={a.redirectUri} hint={a.redirectUriHint} value={list.redirectUri} />
          <Address label={a.logoutUri} hint={a.logoutUriHint} value={list.backChannelLogoutUri} />
        </Section>
      )}
    </>
  );
}

/** An address to register at a provider, with a button that copies it. */
function Address({ label, hint, value }: { label: string; hint: string; value: string }) {
  const m = useMessages();
  const a = m.identityProviders.admin;
  const [copied, setCopied] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  return (
    <div className="flex flex-col gap-1">
      <span className="text-sm font-medium">{label}</span>
      <div className="flex flex-wrap items-center gap-2">
        <code className="min-w-0 flex-1 break-all rounded bg-surface px-2 py-1 text-sm select-all">
          {value}
        </code>
        <Button
          ref={button}
          aria-label={format(a.copyLabel, { address: label })}
          onPress={() => {
            if (button.current !== null) {
              void copyText(value, button.current).then(setCopied);
            }
          }}
          className={secondaryButtonClass + " flex shrink-0 items-center gap-1.5"}
        >
          <CopyIcon size={14} aria-hidden="true" />
          {copied ? a.copied : a.copy}
        </Button>
      </div>
      <p className={hintClass}>{hint}</p>
    </div>
  );
}

/** One provider: its name and issuer, whether it offers sign-in, and its links; or its form. */
function ProviderRow({
  provider,
  onChanged,
}: {
  provider: IdentityProviderSettings;
  onChanged: () => void;
}) {
  const m = useMessages();
  const a = m.identityProviders.admin;
  const sync = useSync();
  const dateFormat = useDateFormat(DATE);
  const [editing, setEditing] = useState(false);
  const [removing, setRemoving] = useState(false);
  const many = provider.linkedAccounts > LINKED_COUNTED;
  if (editing) {
    return (
      <ProviderForm
        initial={draftOf(provider)}
        editing
        onCancel={() => {
          setEditing(false);
        }}
        submit={async (draft, replaceSecret) => {
          const patch = providerPatch(provider, draft, replaceSecret);
          if (Object.keys(patch).length > 0) {
            await sync.admin.updateIdentityProvider(provider.id, patch);
          }
          setEditing(false);
          onChanged();
        }}
      />
    );
  }
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 p-3">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{provider.name}</span>
          {!provider.enabled && (
            <span className="rounded border border-line px-1.5 text-xs text-ink-muted">
              {a.off}
            </span>
          )}
        </div>
        <p className="text-sm break-all text-ink-muted">{provider.issuer}</p>
        <p className="text-xs text-ink-muted">
          {many
            ? format(a.linkedAccountsMany, { count: LINKED_COUNTED })
            : format(a.linkedAccounts, { count: provider.linkedAccounts })}
          {" · "}
          {format(a.addedOn, { date: dateFormat.format(new Date(provider.createdAt)) })}
        </p>
      </div>
      <div className="flex items-center gap-1">
        <Button
          aria-label={format(a.editLabel, { name: provider.name })}
          onPress={() => {
            setEditing(true);
          }}
          className={secondaryButtonClass}
        >
          {a.edit}
        </Button>
        <Button
          aria-label={format(a.removeLabel, { name: provider.name })}
          onPress={() => {
            setRemoving(true);
          }}
          className={secondaryButtonClass + " text-danger"}
        >
          {a.remove}
        </Button>
      </div>
      <ConfirmModal
        isOpen={removing}
        onOpenChange={setRemoving}
        heading={format(a.removeHeading, { name: provider.name })}
        explanation={
          many
            ? format(a.removeLinkedMany, { count: LINKED_COUNTED })
            : format(a.removeLinked, { count: provider.linkedAccounts })
        }
        action={a.remove}
        run={async () => {
          await sync.admin.removeIdentityProvider(provider.id);
          onChanged();
        }}
      />
    </div>
  );
}

/** Adds a provider, once the server has read its discovery document. */
function AddProvider({ onAdded }: { onAdded: () => void }) {
  const m = useMessages();
  const sync = useSync();
  // A new key clears the form once a provider is added.
  const [generation, setGeneration] = useState(0);
  return (
    <div className="flex flex-col gap-2">
      <h3 className="text-sm font-semibold">{m.identityProviders.admin.addHeading}</h3>
      <ProviderForm
        key={generation}
        initial={NEW_PROVIDER}
        editing={false}
        submit={async (draft) => {
          await sync.admin.addIdentityProvider(createRequest(draft));
          setGeneration((n) => n + 1);
          onAdded();
        }}
      />
    </div>
  );
}

/**
 * A provider's fields. Adding one takes its secret; editing one offers to replace the secret,
 * which is otherwise kept as it is. Whatever the server refuses (a provider it could not read,
 * an issuer another provider has) is said under the form, as the server put it.
 */
function ProviderForm({
  initial,
  editing,
  submit,
  onCancel,
}: {
  initial: Draft;
  editing: boolean;
  submit: (draft: Draft, replaceSecret: boolean) => Promise<void>;
  onCancel?: () => void;
}) {
  const m = useMessages();
  const a = m.identityProviders.admin;
  const [draft, setDraft] = useState(initial);
  const [replaceSecret, setReplaceSecret] = useState(!editing);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = <K extends keyof Draft>(key: K, value: Draft[K]) => {
    setDraft((d) => ({ ...d, [key]: value }));
  };
  const text = (key: "name" | "issuer" | "clientId", label: string, hint?: string) => (
    <TextField
      value={draft[key]}
      onChange={(value) => {
        set(key, value);
      }}
      isRequired
      {...(key === "issuer" ? { type: "url" as const } : {})}
      autoComplete="off"
      className={fieldClass}
    >
      <Label className={labelClass}>{label}</Label>
      <Input
        className={inputClass + (key === "name" ? "" : " font-mono")}
        spellCheck={key === "name"}
        autoCapitalize={key === "name" ? undefined : "off"}
      />
      {hint !== undefined && (
        <Text slot="description" className={hintClass}>
          {hint}
        </Text>
      )}
      <FieldError className="text-sm text-danger" />
    </TextField>
  );
  return (
    <Form
      aria-label={editing ? format(a.editLabel, { name: initial.name }) : a.addHeading}
      className={"flex flex-col gap-3" + (editing ? " p-3" : "")}
      onSubmit={(event) => {
        event.preventDefault();
        if (pending) {
          return;
        }
        setPending(true);
        setError(null);
        submit(draft, replaceSecret).then(
          () => {
            setPending(false);
          },
          (failure: unknown) => {
            setError(problemText(failure));
            setPending(false);
          },
        );
      }}
    >
      <div className="grid gap-3 sm:grid-cols-2">
        {text("name", a.name, a.nameHint)}
        {text("issuer", a.issuer, a.issuerHint)}
        {text("clientId", a.clientId)}
        {replaceSecret ? (
          <TextField
            value={draft.clientSecret}
            onChange={(value) => {
              set("clientSecret", value);
            }}
            type="password"
            isRequired
            autoComplete="new-password"
            className={fieldClass}
          >
            <Label className={labelClass}>{editing ? a.newSecret : a.clientSecret}</Label>
            <Input className={inputClass + " font-mono"} />
            <Text slot="description" className={hintClass}>
              {a.clientSecretHint}
            </Text>
            <FieldError className="text-sm text-danger" />
          </TextField>
        ) : (
          <div className="flex flex-col items-start gap-1">
            <span className={labelClass}>{a.clientSecret}</span>
            <Button
              onPress={() => {
                setReplaceSecret(true);
              }}
              className={secondaryButtonClass}
            >
              {a.replaceSecret}
            </Button>
            <p className={hintClass}>{a.clientSecretHint}</p>
          </div>
        )}
      </div>
      {editing && replaceSecret && (
        <Button
          onPress={() => {
            setReplaceSecret(false);
            set("clientSecret", "");
          }}
          className={secondaryButtonClass + " self-start"}
        >
          {a.keepSecret}
        </Button>
      )}
      <div className="grid gap-2 sm:grid-cols-2">
        <ChoiceCheckbox
          isSelected={draft.enabled}
          onChange={(value) => {
            set("enabled", value);
          }}
          label={a.enabled}
          hint={a.enabledHint}
        />
        <ChoiceCheckbox
          isSelected={draft.skipInvite}
          onChange={(value) => {
            set("skipInvite", value);
          }}
          label={a.skipInvite}
          hint={a.skipInviteHint}
        />
        <ChoiceCheckbox
          isSelected={draft.enforcesTwoFactor}
          onChange={(value) => {
            set("enforcesTwoFactor", value);
          }}
          label={a.enforcesTwoFactor}
          hint={a.enforcesTwoFactorHint}
        />
        <ChoiceCheckbox
          isSelected={draft.trustEmail}
          onChange={(value) => {
            set("trustEmail", value);
          }}
          label={a.trustEmail}
          hint={a.trustEmailHint}
        />
      </div>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        {onCancel !== undefined && (
          <Button onPress={onCancel} className={secondaryButtonClass}>
            {m.cancel}
          </Button>
        )}
        <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
          {pending ? (editing ? a.saving : a.adding) : editing ? a.save : a.add}
        </Button>
      </div>
    </Form>
  );
}
