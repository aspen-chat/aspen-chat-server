import { normalizeServerUrl } from "@aspen/protocol";
import { useState, type SyntheticEvent } from "react";
import { Button, Form, Input, Label, TextField } from "react-aria-components";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { AspenIcon } from "./AspenIcon";
import { detectShell } from "@/config";
import { ScanCodeButton } from "@/features/qr/ScanCode";
import { router } from "@/router";
import {
  fieldClass,
  inputClass,
  labelClass,
  linkButtonClass,
  outlineButtonClass,
  primaryButtonClass,
} from "./styles";

/**
 * Asks which Aspen deployment to connect to, under Aspen's icon, in the desktop and mobile
 * shells, which have no server of their own: on first launch, and when the user changes it.
 * The field starts empty. Whatever follows the host in the address (a path, a query, a
 * fragment) is dropped, since a deployment is named by its origin (`normalizeServerUrl`).
 * `onCancel`, given when a server is already chosen, goes back to it. A sign-in code, opened as
 * a link (`offered`) or scanned here on a phone from a computer, names its server: the field is
 * filled in with it and the person continues to it themselves, having seen its address, since
 * anyone can make a code naming any server. Its screen opens once they do.
 */
export function ServerForm({
  onSubmit,
  onCancel,
  offered = null,
}: {
  onSubmit: (serverUrl: string) => void;
  onCancel?: () => void;
  /** The server a sign-in code opened as a link names. */
  offered?: string | null;
}) {
  const m = useMessages();
  const [error, setError] = useState<string | null>(null);
  const [scanned, setScanned] = useState<string | null>(null);
  const suggested = scanned ?? offered;
  const [value, setValue] = useState(suggested ?? "");
  const [shownSuggestion, setShownSuggestion] = useState(suggested);
  if (suggested !== shownSuggestion) {
    setShownSuggestion(suggested);
    if (suggested !== null) {
      setValue(suggested);
    }
  }

  function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      onSubmit(normalizeServerUrl(formString(new FormData(event.currentTarget), "server")));
    } catch {
      setError(m.invalidServerUrl);
    }
  }

  return (
    <Form onSubmit={submit} className="flex w-full max-w-sm flex-col gap-4">
      <div className="flex flex-col items-center gap-3 text-center">
        <AspenIcon />
        <h1 className="text-xl font-semibold">{m.welcomeToAspenChat}</h1>
      </div>
      <TextField
        name="server"
        isRequired
        isInvalid={error !== null}
        value={value}
        // An invalid field blocks the form's submission, so a new address clears the error.
        onChange={(next) => {
          setValue(next);
          setError(null);
        }}
        autoComplete="url"
        className={fieldClass}
      >
        <Label className={labelClass}>{m.deploymentUrlLabel}</Label>
        <Input
          inputMode="url"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          placeholder={m.serverPlaceholder}
          className={inputClass}
        />
        {error !== null && <p className="text-sm text-danger">{error}</p>}
        {suggested !== null && error === null && (
          <p role="status" className="text-sm text-ink-muted">
            {format(m.deviceLink.offeredServer, { server: hostOf(suggested) })}
          </p>
        )}
      </TextField>
      <Button type="submit" className={primaryButtonClass}>
        {m.continue}
      </Button>
      {detectShell() === "mobile" && (
        <ScanCodeButton
          label={m.deviceLink.scan}
          hint={m.deviceLink.scanHintSignedOut}
          accept={(link) => {
            if (link.kind !== "deviceLink") {
              return m.deviceLink.notSignInCode;
            }
            void router.navigate({
              to: "/device-link",
              search: { server: link.server, link: link.id },
            });
            setScanned(link.server);
            return null;
          }}
          className={outlineButtonClass}
        />
      )}
      {onCancel !== undefined && (
        <Button onPress={onCancel} className={linkButtonClass + " self-center text-sm"}>
          {m.backToSignIn}
        </Button>
      )}
    </Form>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
