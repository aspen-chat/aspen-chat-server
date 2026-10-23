import { ApiProblemError, type AccountOffer, type PendingAccount } from "@aspen/protocol";
import { useState } from "react";
import { Button, FieldError, Form, Input, Label, Text, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { useEmailPolicy } from "@/features/email/policy";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  linkButtonClass,
  primaryButtonClass,
} from "./styles";

/**
 * The account an identity provider's first sign-in offers to make: a username, prefilled with
 * the provider's when an account could take it, and a registration invite and an email address
 * only where the offer asks for them. A name someone holds leaves the offer standing for
 * another; the session the account comes with is stored, and the signed-in app takes over.
 */
export function ProviderAccountStep({
  offer,
  account,
  onCancel,
}: {
  offer: AccountOffer;
  account: PendingAccount;
  /** Gives up on the offer, back to signing in. */
  onCancel: () => void;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const policy = useEmailPolicy();
  const [pending, setPending] = useState(false);
  const [usernameError, setUsernameError] = useState<string | null>(null);
  const [inviteError, setInviteError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [email, setEmail] = useState("");
  const [newsletter, setNewsletter] = useState(false);
  const providerEmail = offer.email ?? null;
  // The provider's verified address is the account's where the deployment takes it; otherwise
  // one is asked for as registration asks, required where the offer says so.
  const askEmail = providerEmail === null && (offer.emailRequired || policy?.available === true);
  const offerNewsletter =
    policy?.newsletter === true && (providerEmail !== null || email.trim() !== "");

  async function submit(form: HTMLFormElement) {
    const data = new FormData(form);
    const name = formString(data, "username").trim();
    const invite = offer.inviteRequired ? formString(data, "invite").trim() : "";
    const address = askEmail ? email.trim() : "";
    setUsernameError(null);
    setInviteError(null);
    setError(null);
    setPending(true);
    try {
      await client.createIdentityProviderAccount(account, {
        name,
        ...(invite === "" ? {} : { inviteCode: invite }),
        ...(address === "" ? {} : { email: address }),
        ...(offerNewsletter ? { newsletter } : {}),
      });
    } catch (e) {
      if (!(e instanceof ApiProblemError)) {
        setError(String(e));
      } else if (e.code === "usernameTaken") {
        setUsernameError(e.message);
      } else if (
        e.code === "registrationInviteRequired" ||
        e.code === "registrationInviteInvalid"
      ) {
        setInviteError(e.message);
      } else if (e.code === "validation" && askEmail) {
        // The username's rules and the address's both answer `validation`, and only the server's
        // text says which; with an address asked for, it shows for the whole form.
        setError(e.message);
      } else if (e.code === "validation") {
        setUsernameError(e.message);
      } else {
        setError(e.message);
      }
    } finally {
      setPending(false);
    }
  }

  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        void submit(event.currentTarget);
      }}
      className="flex w-full max-w-sm flex-col gap-4"
    >
      <h1 className="text-2xl font-semibold">{m.identityProviders.accountHeading}</h1>
      <p className={hintClass}>
        {format(m.identityProviders.accountHint, { provider: offer.providerName })}
      </p>
      {offer.displayName != null && (
        <p className="text-sm">
          {format(m.identityProviders.displayNameNote, { name: offer.displayName })}
        </p>
      )}
      {offer.inviteRequired && (
        <TextField
          name="invite"
          // A field marked invalid blocks the form's next submission, so its error goes as soon
          // as the field is edited.
          onChange={() => {
            setInviteError(null);
          }}
          isRequired
          autoComplete="off"
          isInvalid={inviteError !== null}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.inviteCodeLabel}</Label>
          <Input className={inputClass + " font-mono"} spellCheck={false} autoCapitalize="off" />
          {inviteError === null ? (
            <Text slot="description" className={hintClass}>
              {m.inviteCodeRequiredHint}
            </Text>
          ) : (
            <FieldError className="text-sm text-danger">{inviteError}</FieldError>
          )}
        </TextField>
      )}
      <TextField
        name="username"
        defaultValue={offer.suggestedName ?? ""}
        onChange={() => {
          setUsernameError(null);
        }}
        isRequired
        autoFocus
        maxLength={32}
        autoComplete="username"
        isInvalid={usernameError !== null}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.usernameLabel}</Label>
        <Input className={inputClass} spellCheck={false} autoCapitalize="off" />
        <FieldError className="text-sm text-danger">{usernameError}</FieldError>
      </TextField>
      {providerEmail !== null && (
        <p className="text-sm">
          {format(m.identityProviders.emailNote, {
            email: providerEmail,
            provider: offer.providerName,
          })}
        </p>
      )}
      {askEmail && (
        <TextField
          name="email"
          type="email"
          value={email}
          onChange={setEmail}
          isRequired={offer.emailRequired}
          autoComplete="email"
          className={fieldClass}
        >
          <Label className={labelClass}>{m.email.addressLabel}</Label>
          <Input className={inputClass} spellCheck={false} autoCapitalize="off" />
          <Text slot="description" className={hintClass}>
            {offer.emailRequired ? m.email.registerRequiredHint : m.email.registerOptionalHint}
          </Text>
          <FieldError className="text-sm text-danger" />
        </TextField>
      )}
      {offerNewsletter && (
        <ChoiceCheckbox
          isSelected={newsletter}
          onChange={setNewsletter}
          label={m.email.newsletterSubscribe}
          hint={m.email.newsletterSubscribeHint}
        />
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
        {pending ? m.registering : m.register}
      </Button>
      <Button onPress={onCancel} className={linkButtonClass + " self-start text-sm"}>
        {m.identityProviders.signInAnotherWay}
      </Button>
    </Form>
  );
}
