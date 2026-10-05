import {
  ApiProblemError,
  PasskeyCancelledError,
  type PasskeyTransport,
  type SecondFactorMethod,
  type TypedSecondFactor,
} from "@aspen/protocol";
import { KeyIcon } from "@phosphor-icons/react";
import { useState, type SyntheticEvent } from "react";
import { Button, FieldError, Form, Input, Label, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { OtherDeviceSignIn } from "./OtherDeviceSignIn";
import { usePasskeyTransport } from "./passkeyTransport";
import { SecondFactorFields } from "./SecondFactorFields";
import { PasswordReset } from "@/features/email/PasswordReset";
import { useEmailPolicy } from "@/features/email/policy";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  linkButtonClass,
  outlineButtonClass,
  primaryButtonClass,
} from "./styles";

type Step =
  | { kind: "password" }
  | { kind: "secondFactor"; ticket: string; methods: SecondFactorMethod[] }
  | { kind: "reset" };

/** Why an attempt failed, as the form shows it; `null` when the user backed out. */
function failure(e: unknown): string | null {
  if (e instanceof PasskeyCancelledError) {
    return null;
  }
  return e instanceof ApiProblemError ? e.message : String(e);
}

/**
 * Signing in to the currently selected server: a username and password, then a second factor
 * when the account has two-factor sign-in on; or a passkey on its own; or another device, by a
 * QR code (`OtherDeviceSignIn`). Where the deployment sends email, a forgotten password is reset
 * by a code mailed to the account's address (`PasswordReset`). Failures show the
 * server's localized Problem text. Which server it is shows above it (`DeploymentWelcome`).
 */
export function LoginForm({ onSwitchToRegister }: { onSwitchToRegister: () => void }) {
  const transport = usePasskeyTransport();
  const [step, setStep] = useState<Step>({ kind: "password" });
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const m = useMessages();

  if (step.kind === "reset") {
    return (
      <PasswordReset
        onDone={() => {
          setNotice(m.email.resetDone);
          setStep({ kind: "password" });
        }}
        onCancel={() => {
          setStep({ kind: "password" });
        }}
      />
    );
  }
  if (step.kind === "secondFactor") {
    return (
      <SecondFactorStep
        ticket={step.ticket}
        methods={step.methods}
        transport={transport}
        onStartOver={(reason) => {
          setError(reason);
          setStep({ kind: "password" });
        }}
      />
    );
  }
  return (
    <PasswordStep
      transport={transport}
      error={error}
      setError={setError}
      notice={notice}
      onForgotPassword={() => {
        setError(null);
        setNotice(null);
        setStep({ kind: "reset" });
      }}
      onSecondFactor={(ticket, methods) => {
        setError(null);
        setStep({ kind: "secondFactor", ticket, methods });
      }}
      onSwitchToRegister={onSwitchToRegister}
    />
  );
}

function PasswordStep({
  transport,
  error,
  setError,
  notice,
  onForgotPassword,
  onSecondFactor,
  onSwitchToRegister,
}: {
  transport: PasskeyTransport | null;
  error: string | null;
  setError: (error: string | null) => void;
  /** Something done that the reader returned from, such as a password reset. */
  notice: string | null;
  onForgotPassword: () => void;
  onSecondFactor: (ticket: string, methods: SecondFactorMethod[]) => void;
  onSwitchToRegister: () => void;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const [pending, setPending] = useState<"password" | "passkey" | null>(null);
  const emailAvailable = useEmailPolicy()?.available === true;

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    setPending("password");
    setError(null);
    try {
      const outcome = await client.login(
        formString(data, "username"),
        formString(data, "password"),
      );
      if (outcome.status === "secondFactorRequired") {
        onSecondFactor(outcome.ticket, outcome.methods);
      }
    } catch (e) {
      setError(failure(e));
    } finally {
      setPending(null);
    }
  }

  async function passkey(transport: PasskeyTransport) {
    setPending("passkey");
    setError(null);
    try {
      await client.runPasskeyCeremony({ purpose: "signIn" }, transport);
    } catch (e) {
      setError(failure(e));
    } finally {
      setPending(null);
    }
  }

  return (
    <div className="flex w-full max-w-sm flex-col gap-4">
      <Form
        onSubmit={(e) => {
          void submit(e);
        }}
        className="flex flex-col gap-4"
      >
        <h1 className="text-2xl font-semibold">{m.loginHeading}</h1>
        <TextField
          name="username"
          isRequired
          autoComplete="username webauthn"
          className={fieldClass}
        >
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
        {notice !== null && error === null && (
          <p role="status" className="rounded-md bg-accent-soft px-3 py-2 text-sm">
            {notice}
          </p>
        )}
        {error !== null && (
          <p role="alert" className={alertClass}>
            {error}
          </p>
        )}
        <Button type="submit" isDisabled={pending !== null} className={primaryButtonClass}>
          {pending === "password" ? m.signingIn : m.signIn}
        </Button>
        {emailAvailable && (
          <Button onPress={onForgotPassword} className={linkButtonClass + " self-start text-sm"}>
            {m.email.forgotPassword}
          </Button>
        )}
      </Form>
      <div className="flex items-center gap-3 text-xs text-ink-muted" aria-hidden="true">
        <span className="h-px flex-1 bg-line" />
        {m.orDivider}
        <span className="h-px flex-1 bg-line" />
      </div>
      {transport !== null && (
        <>
          <Button
            isDisabled={pending !== null}
            onPress={() => {
              void passkey(transport);
            }}
            className={outlineButtonClass}
          >
            <KeyIcon size={18} aria-hidden="true" />
            {pending === "passkey"
              ? transport.kind === "handoff"
                ? m.passkeyWaitingBrowser
                : m.passkeyWaiting
              : m.signInWithPasskey}
          </Button>
        </>
      )}
      <OtherDeviceSignIn className={outlineButtonClass} />
      <p className="text-sm text-ink-muted">
        {m.noAccountYet}{" "}
        <Button onPress={onSwitchToRegister} className={linkButtonClass}>
          {m.createAccount}
        </Button>
      </p>
    </div>
  );
}

function SecondFactorStep({
  ticket,
  methods,
  transport,
  onStartOver,
}: {
  ticket: string;
  methods: SecondFactorMethod[];
  transport: PasskeyTransport | null;
  onStartOver: (reason: string | null) => void;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const totpAvailable = methods.includes("totp");
  const recoveryAvailable = methods.includes("recoveryCode");
  const passkeyAvailable = methods.includes("passkey") && transport !== null;
  const [method, setMethod] = useState<TypedSecondFactor>(totpAvailable ? "totp" : "recoveryCode");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function attempt(run: () => Promise<unknown>) {
    setPending(true);
    setError(null);
    try {
      await run();
    } catch (e) {
      // The ticket expired or was used: the password has to be given again.
      if (e instanceof ApiProblemError && e.code === "invalidToken") {
        onStartOver(e.message);
        return;
      }
      setError(failure(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        const code = formString(new FormData(event.currentTarget), "code");
        void attempt(() => client.completeSecondFactor(ticket, method, code));
      }}
      className="flex w-full max-w-sm flex-col gap-4"
    >
      <h1 className="text-2xl font-semibold">{m.twoFactor.heading}</h1>
      {(totpAvailable || recoveryAvailable) && (
        <SecondFactorFields
          method={method}
          onMethodChange={(next) => {
            setError(null);
            setMethod(next);
          }}
          totpAvailable={totpAvailable}
          recoveryAvailable={recoveryAvailable}
        />
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {(totpAvailable || recoveryAvailable) && (
        <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
          {pending ? m.twoFactor.verifying : m.twoFactor.verify}
        </Button>
      )}
      {passkeyAvailable && (
        <Button
          isDisabled={pending}
          onPress={() => {
            void attempt(() => client.runPasskeyCeremony({ purpose: "signIn", ticket }, transport));
          }}
          className={outlineButtonClass}
        >
          <KeyIcon size={18} aria-hidden="true" />
          {m.twoFactor.usePasskey}
        </Button>
      )}
      <Button
        onPress={() => {
          onStartOver(null);
        }}
        className={linkButtonClass + " self-start text-sm"}
      >
        {m.twoFactor.startOver}
      </Button>
    </Form>
  );
}
