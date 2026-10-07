import { ApiProblemError } from "@aspen/protocol";
import { useState } from "react";
import { Button, FieldError, Form, Input, Label, Text, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { problemText } from "@/api/problemText";
import { PASSWORD_MIN_LENGTH } from "@/features/auth/password";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  linkButtonClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

type Step =
  | { kind: "username" }
  | { kind: "address"; id: string; masked: string }
  | { kind: "code"; id: string; masked: string };

/**
 * Resetting a forgotten password by email, from the sign-in screen: the username, then the
 * account's address typed whole (the server shows it masked: its first three characters before
 * the `@`, and the domain), then the code mailed there with a new password. The server answers
 * the address the same whether or not it is the account's, so the code step says a code was sent
 * only if it was. A reset that ends (expired, used, or too many tries) starts again from the
 * username, saying why. Finished, it returns to signing in with `onDone`.
 */
export function PasswordReset({ onDone, onCancel }: { onDone: () => void; onCancel: () => void }) {
  const m = useMessages();
  const client = useAspenClient();
  const [step, setStep] = useState<Step>({ kind: "username" });
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fieldError, setFieldError] = useState<{ field: string; text: string } | null>(null);

  /** Runs one step, sending the reset back to its start when the server ended it. */
  async function attempt(run: () => Promise<void>, field: string) {
    setPending(true);
    setError(null);
    setFieldError(null);
    try {
      await run();
    } catch (e) {
      if (
        e instanceof ApiProblemError &&
        (e.code === "passwordResetExpired" || e.code === "tooManyAttempts") &&
        step.kind !== "username"
      ) {
        setStep({ kind: "username" });
        setError(e.message);
      } else if (e instanceof ApiProblemError && e.code === "passwordResetUnavailable") {
        setError(e.message);
      } else {
        setFieldError({ field, text: problemText(e) });
      }
    } finally {
      setPending(false);
    }
  }

  const clear = () => {
    setFieldError(null);
  };
  const errorOf = (field: string) => (fieldError?.field === field ? fieldError.text : null);

  return (
    <div className="flex w-full max-w-sm flex-col gap-4">
      <h1 className="text-2xl font-semibold">{m.email.resetHeading}</h1>
      {step.kind === "username" && (
        <Form
          onSubmit={(event) => {
            event.preventDefault();
            const username = formString(new FormData(event.currentTarget), "username").trim();
            void attempt(async () => {
              const started = await client.startPasswordReset(username);
              setStep({ kind: "address", id: started.id, masked: started.maskedAddress });
            }, "username");
          }}
          className="flex flex-col gap-4"
        >
          <p className={hintClass}>{m.email.resetUsernamePrompt}</p>
          <TextField
            name="username"
            isRequired
            autoComplete="username"
            onChange={clear}
            isInvalid={errorOf("username") !== null}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.usernameLabel}</Label>
            <Input className={inputClass} />
            <FieldError className="text-sm text-danger">{errorOf("username")}</FieldError>
          </TextField>
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
            {pending ? m.email.resetWorking : m.email.resetContinue}
          </Button>
        </Form>
      )}
      {step.kind === "address" && (
        <Form
          onSubmit={(event) => {
            event.preventDefault();
            const address = formString(new FormData(event.currentTarget), "address").trim();
            void attempt(async () => {
              await client.sendPasswordResetCode(step.id, address);
              setStep({ kind: "code", id: step.id, masked: step.masked });
            }, "address");
          }}
          className="flex flex-col gap-4"
        >
          <p className={hintClass}>{format(m.email.resetAddressPrompt, { masked: step.masked })}</p>
          <TextField
            name="address"
            type="email"
            isRequired
            autoComplete="email"
            onChange={clear}
            isInvalid={errorOf("address") !== null}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.email.addressLabel}</Label>
            <Input className={inputClass} spellCheck={false} autoCapitalize="off" />
            <FieldError className="text-sm text-danger">{errorOf("address")}</FieldError>
          </TextField>
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
            {pending ? m.email.resetWorking : m.email.resetSendCode}
          </Button>
        </Form>
      )}
      {step.kind === "code" && (
        <Form
          onSubmit={(event) => {
            event.preventDefault();
            const data = new FormData(event.currentTarget);
            const code = formString(data, "code").trim();
            const password = formString(data, "newPassword");
            if (password.length < PASSWORD_MIN_LENGTH) {
              setFieldError({ field: "newPassword", text: m.passwordHint });
              return;
            }
            if (password !== formString(data, "confirmPassword")) {
              setFieldError({ field: "confirmPassword", text: m.passwordsDoNotMatch });
              return;
            }
            void attempt(async () => {
              await client.completePasswordReset(step.id, code, password);
              onDone();
            }, "code");
          }}
          className="flex flex-col gap-4"
        >
          <p className={hintClass}>{format(m.email.resetCodePrompt, { masked: step.masked })}</p>
          <TextField
            name="code"
            isRequired
            inputMode="numeric"
            autoComplete="one-time-code"
            maxLength={8}
            onChange={clear}
            isInvalid={errorOf("code") !== null}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.email.codeLabel}</Label>
            <Input className={inputClass + " font-mono tracking-widest"} />
            <FieldError className="text-sm text-danger">{errorOf("code")}</FieldError>
          </TextField>
          <TextField
            name="newPassword"
            type="password"
            isRequired
            autoComplete="new-password"
            onChange={clear}
            isInvalid={errorOf("newPassword") !== null}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.email.resetNewPassword}</Label>
            <Input className={inputClass} />
            {errorOf("newPassword") === null ? (
              <Text slot="description" className={hintClass}>
                {m.passwordHint}
              </Text>
            ) : (
              <FieldError className="text-sm text-danger">{errorOf("newPassword")}</FieldError>
            )}
          </TextField>
          <TextField
            name="confirmPassword"
            type="password"
            isRequired
            autoComplete="new-password"
            onChange={clear}
            isInvalid={errorOf("confirmPassword") !== null}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.confirmPasswordLabel}</Label>
            <Input className={inputClass} />
            <FieldError className="text-sm text-danger">{errorOf("confirmPassword")}</FieldError>
          </TextField>
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
            {pending ? m.email.resetWorking : m.email.resetFinish}
          </Button>
          <Button
            isDisabled={pending}
            onPress={() => {
              setFieldError(null);
              setStep({ kind: "address", id: step.id, masked: step.masked });
            }}
            className={linkButtonClass + " self-start text-sm"}
          >
            {m.email.resetNoCode}
          </Button>
        </Form>
      )}
      <Button onPress={onCancel} className={linkButtonClass + " self-start text-sm"}>
        {m.email.resetBack}
      </Button>
    </div>
  );
}
