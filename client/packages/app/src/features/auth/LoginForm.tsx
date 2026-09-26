import { ApiProblemError } from "@aspen/protocol";
import { useState, type SyntheticEvent } from "react";
import { Button, FieldError, Form, Input, Label, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { fieldClass, inputClass, labelClass, linkButtonClass, primaryButtonClass } from "./styles";

/**
 * Username and password against the currently selected server. Failures show the server's
 * localized Problem text.
 */
export function LoginForm({
  serverUrl,
  onChangeServer,
  onSwitchToRegister,
}: {
  serverUrl: string;
  onChangeServer: () => void;
  onSwitchToRegister: () => void;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    setPending(true);
    setError(null);
    try {
      await client.login(formString(data, "username"), formString(data, "password"));
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
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
      <h1 className="text-2xl font-semibold">{m.loginHeading}</h1>
      <div className="flex items-baseline justify-between text-sm text-ink-muted">
        <span>
          {m.serverLabel}: <span className="font-mono">{serverUrl}</span>
        </span>
        <Button onPress={onChangeServer} className={linkButtonClass}>
          {m.changeServer}
        </Button>
      </div>
      <TextField name="username" isRequired autoComplete="username" className={fieldClass}>
        <Label className={labelClass}>{m.usernameLabel}</Label>
        <Input className={inputClass} />
        <FieldError className="text-sm text-danger" />
      </TextField>
      <TextField
        name="password"
        type="password"
        isRequired
        autoComplete="current-password"
        className={fieldClass}
      >
        <Label className={labelClass}>{m.passwordLabel}</Label>
        <Input className={inputClass} />
        <FieldError className="text-sm text-danger" />
      </TextField>
      {error !== null && (
        <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
        {pending ? m.signingIn : m.signIn}
      </Button>
      <p className="text-sm text-ink-muted">
        {m.noAccountYet}{" "}
        <Button onPress={onSwitchToRegister} className={linkButtonClass}>
          {m.createAccount}
        </Button>
      </p>
    </Form>
  );
}
