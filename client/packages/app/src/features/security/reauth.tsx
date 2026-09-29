import {
  ApiProblemError,
  PasskeyCancelledError,
  type PasskeyTransport,
  type TypedSecondFactor,
} from "@aspen/protocol";
import { useCallback, useRef, useState, type ReactNode } from "react";
import {
  Button,
  Dialog,
  FieldError,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  TextField,
} from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { SecondFactorFields } from "@/features/auth/SecondFactorFields";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { formString } from "@/forms";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import type { SecuritySettings } from "./api";
import { ReauthContext, type WithReauth } from "./reauthContext";

function needsReauth(e: unknown): boolean {
  return e instanceof ApiProblemError && e.code === "reauthenticationRequired";
}

export function ReauthProvider({
  settings,
  transport,
  children,
}: {
  settings: SecuritySettings | null;
  transport: PasskeyTransport | null;
  children: ReactNode;
}) {
  const [asking, setAsking] = useState(false);
  const answer = useRef<((confirmed: boolean) => void) | null>(null);
  const withReauth = useCallback<WithReauth>(async (action) => {
    try {
      return await action();
    } catch (e) {
      if (!needsReauth(e)) {
        throw e;
      }
    }
    const confirmed = await new Promise<boolean>((resolve) => {
      answer.current = resolve;
      setAsking(true);
    });
    if (!confirmed) {
      return undefined;
    }
    return action();
  }, []);
  const finish = (confirmed: boolean) => {
    setAsking(false);
    answer.current?.(confirmed);
    answer.current = null;
  };
  return (
    <ReauthContext.Provider value={withReauth}>
      {children}
      {asking && settings !== null && (
        <ReauthDialog settings={settings} transport={transport} onFinish={finish} />
      )}
    </ReauthContext.Provider>
  );
}

function ReauthDialog({
  settings,
  transport,
  onFinish,
}: {
  settings: SecuritySettings;
  transport: PasskeyTransport | null;
  onFinish: (confirmed: boolean) => void;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const twoFactor = settings.twoFactorEnabled;
  const [method, setMethod] = useState<TypedSecondFactor>(settings.totp ? "totp" : "recoveryCode");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function attempt(run: () => Promise<unknown>) {
    setPending(true);
    setError(null);
    try {
      await run();
      onFinish(true);
    } catch (e) {
      if (!(e instanceof PasskeyCancelledError)) {
        setError(e instanceof ApiProblemError ? e.message : String(e));
      }
    } finally {
      setPending(false);
    }
  }

  const passkeyAvailable = twoFactor && settings.passkeys.length > 0 && transport !== null;
  const codesAvailable = settings.totp || settings.recoveryCodesRemaining > 0;
  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(open) => {
        if (!open) {
          onFinish(false);
        }
      }}
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass} aria-label={m.security.reauthHeading}>
          <DialogHeading>{m.security.reauthHeading}</DialogHeading>
          <Form
            className="flex flex-col gap-3"
            onSubmit={(event) => {
              event.preventDefault();
              const data = new FormData(event.currentTarget);
              void attempt(() =>
                twoFactor
                  ? client.reauthenticate(method, formString(data, "code"))
                  : client.reauthenticate("password", formString(data, "password")),
              );
            }}
          >
            {twoFactor ? (
              codesAvailable && (
                <SecondFactorFields
                  method={method}
                  onMethodChange={setMethod}
                  totpAvailable={settings.totp}
                  recoveryAvailable={settings.recoveryCodesRemaining > 0}
                />
              )
            ) : (
              <>
                <p className={hintClass}>{m.security.reauthPassword}</p>
                <TextField
                  name="password"
                  type="password"
                  isRequired
                  autoFocus
                  autoComplete="current-password"
                  className={fieldClass}
                >
                  <Label className={labelClass}>{m.passwordLabel}</Label>
                  <Input className={inputClass} />
                  <FieldError className="text-sm text-danger" />
                </TextField>
              </>
            )}
            {error !== null && (
              <p role="alert" className={alertClass}>
                {error}
              </p>
            )}
            <div className="flex flex-wrap items-center justify-end gap-2">
              {passkeyAvailable && (
                <Button
                  isDisabled={pending}
                  onPress={() => {
                    void attempt(() =>
                      client.runPasskeyCeremony({ purpose: "reauthenticate" }, transport),
                    );
                  }}
                  className={secondaryButtonClass + " me-auto"}
                >
                  {m.twoFactor.usePasskey}
                </Button>
              )}
              {(!twoFactor || codesAvailable) && (
                <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
                  {pending ? m.twoFactor.verifying : m.security.confirm}
                </Button>
              )}
            </div>
          </Form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
