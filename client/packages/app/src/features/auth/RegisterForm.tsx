import { ApiProblemError } from "@aspen/protocol";
import { useEffect, useState, type SyntheticEvent } from "react";
import { Button, FieldError, Form, Input, Label, Text, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { useAuthMethods } from "@/features/auth/authMethods";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { fieldClass, inputClass, labelClass, linkButtonClass, primaryButtonClass } from "./styles";
import { PASSWORD_MIN_LENGTH } from "@/features/auth/password";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";

/**
 * Creates an account, then signs in with the same credentials so the user lands in the app
 * without typing them twice. Server Problems are attached to the field they concern.
 */
export function RegisterForm({
  onSwitchToLogin,
  initialInvite,
}: {
  onSwitchToLogin: () => void;
  /** The invite code a link brought, filled in. */
  initialInvite?: string | undefined;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const methods = useAuthMethods();
  const inviteRequired = methods?.registrationInviteRequired === true;
  // Asked for when the server requires one, or when a link brought one to check.
  const askInvite = inviteRequired || initialInvite !== undefined;
  const [inviteError, setInviteError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [usernameError, setUsernameError] = useState<string | null>(null);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [confirmError, setConfirmError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const joins = useDualInviteCommunity(initialInvite);

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const username = formString(data, "username");
    const password = formString(data, "password");
    const confirm = formString(data, "confirmPassword");
    const invite = askInvite ? formString(data, "invite").trim() : "";
    setInviteError(null);
    setUsernameError(null);
    setPasswordError(null);
    setConfirmError(null);
    setError(null);
    if (password.length < PASSWORD_MIN_LENGTH) {
      setPasswordError(m.passwordHint);
      return;
    }
    if (password !== confirm) {
      setConfirmError(m.passwordsDoNotMatch);
      return;
    }
    setPending(true);
    try {
      await client.register(username, password, invite === "" ? undefined : invite);
      await client.login(username, password);
    } catch (e) {
      if (e instanceof ApiProblemError) {
        switch (e.code) {
          case "usernameTaken":
          case "validation":
            setUsernameError(e.message);
            break;
          case "passwordRequirementsNotMet":
            setPasswordError(e.message);
            break;
          case "registrationInviteRequired":
          case "registrationInviteInvalid":
            setInviteError(e.message);
            break;
          default:
            setError(e.message);
        }
      } else {
        setError(String(e));
      }
    } finally {
      setPending(false);
    }
  }

  return (
    <Form
      onSubmit={(e) => {
        void submit(e);
      }}
      className="flex w-full max-w-sm flex-col gap-4"
    >
      <h1 className="text-2xl font-semibold">{m.registerHeading}</h1>
      {joins === undefined ? (
        <div aria-busy="true">
          <LoadingLabel />
          <Skeleton className="h-5 w-3/4" />
        </div>
      ) : (
        joins !== null && (
          <p className="text-sm text-ink-muted">
            {format(m.qr.registerJoins, { community: joins })}
          </p>
        )
      )}
      {askInvite && (
        <TextField
          name="invite"
          // A field marked invalid blocks the form's next submission, so its error goes as soon
          // as the field is edited.
          onChange={() => {
            setInviteError(null);
          }}
          isRequired={inviteRequired}
          {...(initialInvite === undefined ? {} : { defaultValue: initialInvite })}
          autoComplete="off"
          isInvalid={inviteError !== null}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.inviteCodeLabel}</Label>
          <Input className={inputClass + " font-mono"} spellCheck={false} autoCapitalize="off" />
          {inviteError === null ? (
            <Text slot="description" className="text-sm text-ink-muted">
              {inviteRequired ? m.inviteCodeRequiredHint : m.inviteCodeHint}
            </Text>
          ) : (
            <FieldError className="text-sm text-danger">{inviteError}</FieldError>
          )}
        </TextField>
      )}
      <TextField
        name="username"
        // A field marked invalid blocks the form's next submission, so its error goes as soon
        // as the field is edited.
        onChange={() => {
          setUsernameError(null);
        }}
        isRequired
        maxLength={32}
        autoComplete="username"
        isInvalid={usernameError !== null}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.usernameLabel}</Label>
        <Input className={inputClass} />
        <FieldError className="text-sm text-danger">{usernameError}</FieldError>
      </TextField>
      <TextField
        name="password"
        // A field marked invalid blocks the form's next submission, so its error goes as soon
        // as the field is edited.
        onChange={() => {
          setPasswordError(null);
        }}
        type="password"
        isRequired
        autoComplete="new-password"
        isInvalid={passwordError !== null}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.passwordLabel}</Label>
        <Input className={inputClass} />
        {passwordError === null ? (
          <Text slot="description" className="text-sm text-ink-muted">
            {m.passwordHint}
          </Text>
        ) : (
          <FieldError className="text-sm text-danger">{passwordError}</FieldError>
        )}
      </TextField>
      <TextField
        name="confirmPassword"
        // A field marked invalid blocks the form's next submission, so its error goes as soon
        // as the field is edited.
        onChange={() => {
          setConfirmError(null);
        }}
        type="password"
        isRequired
        autoComplete="new-password"
        isInvalid={confirmError !== null}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.confirmPasswordLabel}</Label>
        <Input className={inputClass} />
        <FieldError className="text-sm text-danger">{confirmError}</FieldError>
      </TextField>
      {error !== null && (
        <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
        {pending ? m.registering : m.register}
      </Button>
      <p className="text-sm text-ink-muted">
        {m.haveAccount}{" "}
        <Button onPress={onSwitchToLogin} className={linkButtonClass}>
          {m.signInInstead}
        </Button>
      </p>
    </Form>
  );
}

/**
 * The name of the community a dual invite's account joins as it is made, read before anyone signs
 * in; `null` for no invite, a plain one, or one that no longer works, and `undefined` while it is
 * read.
 */
function useDualInviteCommunity(invite: string | undefined): string | null | undefined {
  const client = useAspenClient();
  const [name, setName] = useState<string | null | undefined>(
    invite === undefined ? null : undefined,
  );
  useEffect(() => {
    if (invite === undefined) {
      return;
    }
    let live = true;
    client.registrationInvite(invite).then(
      (read) => {
        const community = read.included.communities?.[0];
        if (live) {
          setName(read.data.communityInvite != null ? (community?.name ?? null) : null);
        }
      },
      () => {
        if (live) {
          setName(null);
        }
      },
    );
    return () => {
      live = false;
    };
  }, [client, invite]);
  return name;
}
