import { ApiProblemError } from "@aspen/protocol";
import { useState, type SyntheticEvent } from "react";
import { Button, FieldError, Form, Input, Label, Text, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { fieldClass, inputClass, labelClass, linkButtonClass, primaryButtonClass } from "./styles";

/**
 * Mirrors the server's password rule so the form can refuse obviously short passwords before a
 * round trip. The server remains the authority and reports `passwordRequirementsNotMet` when
 * the two disagree.
 */
const PASSWORD_MIN_LENGTH = 8;

/**
 * Creates an account, then signs in with the same credentials so the user lands in the app
 * without typing them twice. Server Problems are attached to the field they concern.
 */
export function RegisterForm({ onSwitchToLogin }: { onSwitchToLogin: () => void }) {
  const m = useMessages();
  const client = useAspenClient();
  const [pending, setPending] = useState(false);
  const [usernameError, setUsernameError] = useState<string | null>(null);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [confirmError, setConfirmError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const username = formString(data, "username");
    const password = formString(data, "password");
    const confirm = formString(data, "confirmPassword");
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
      await client.register(username, password);
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
      <TextField
        name="username"
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
