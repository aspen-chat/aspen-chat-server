import type { TypedSecondFactor } from "@aspen/protocol";
import { Button, FieldError, Input, Label, TextField } from "react-aria-components";
import { useMessages } from "@/i18n/context";
import { fieldClass, hintClass, inputClass, labelClass, linkButtonClass } from "./styles";

/**
 * The code a second factor is typed as: an authenticator code, or one of the recovery codes,
 * with a switch between them. The field is named `code`. Signing in and re-verifying both use
 * it.
 */
export function SecondFactorFields({
  method,
  onMethodChange,
  recoveryAvailable,
  totpAvailable,
}: {
  method: TypedSecondFactor;
  onMethodChange: (method: TypedSecondFactor) => void;
  totpAvailable: boolean;
  recoveryAvailable: boolean;
}) {
  const m = useMessages();
  const totp = method === "totp";
  const other: TypedSecondFactor = totp ? "recoveryCode" : "totp";
  const canSwitch = totp ? recoveryAvailable : totpAvailable;
  return (
    <>
      <p className={hintClass}>{totp ? m.twoFactor.totpPrompt : m.twoFactor.recoveryPrompt}</p>
      <TextField
        // A new key per method clears what was typed for the other.
        key={method}
        name="code"
        isRequired
        autoFocus
        autoComplete="one-time-code"
        inputMode={totp ? "numeric" : "text"}
        className={fieldClass}
      >
        <Label className={labelClass}>
          {totp ? m.twoFactor.totpLabel : m.twoFactor.recoveryLabel}
        </Label>
        <Input
          className={inputClass + (totp ? " font-mono tracking-widest" : " font-mono")}
          spellCheck={false}
          autoCapitalize="off"
        />
        <FieldError className="text-sm text-danger" />
      </TextField>
      {canSwitch && (
        <Button
          onPress={() => {
            onMethodChange(other);
          }}
          className={linkButtonClass + " self-start text-sm"}
        >
          {totp ? m.twoFactor.useRecoveryCode : m.twoFactor.useAuthenticator}
        </Button>
      )}
    </>
  );
}
