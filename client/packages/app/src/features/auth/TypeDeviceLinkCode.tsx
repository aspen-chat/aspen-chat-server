import { ApiProblemError } from "@aspen/protocol";
import { KeyboardIcon } from "@phosphor-icons/react";
import { useState, type SyntheticEvent } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  TextField,
} from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useOptionalReauth } from "@/features/security/reauthContext";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { DeviceLinkScreen } from "./DeviceLinkScreen";
import { rememberTyped } from "./takenLinks";
import { thisDeviceName } from "./deviceName";
import { fieldClass, hintClass, inputClass, labelClass, primaryButtonClass } from "./styles";
import { normalizeUserCode } from "./userCode";

/**
 * Taking a sign-in code by typing the user code shown beside its QR code, for a device that
 * cannot scan it, or whose person would rather type. Signed in, it gives this account to the
 * device asking; signed out, it takes an account offered. Once taken, the dialog goes on as a
 * scanned code's screen does (`DeviceLinkScreen`). A code that cannot be one is refused here,
 * unsent, since the server counts every code that matches nothing against whoever typed it.
 */
export function TypeDeviceLinkCode({ className }: { className: string }) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Button className={className}>
        <KeyboardIcon size={18} aria-hidden="true" />
        {m.deviceLink.typeInstead}
      </Button>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => (
              <>
                <DialogHeading>{m.deviceLink.typeHeading}</DialogHeading>
                <Steps close={close} />
              </>
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function Steps({ close }: { close: () => void }) {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useOptionalReauth();
  const signedIn = client.session !== null;
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [taken, setTaken] = useState<string | null>(null);

  if (taken !== null) {
    return <DeviceLinkScreen server={client.baseUrl} id={taken} onLeave={close} />;
  }

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const code = normalizeUserCode(value);
    if (code === null) {
      setError(m.deviceLink.typeMalformed);
      return;
    }
    setPending(true);
    try {
      const name = thisDeviceName((app, system) =>
        system === null ? app : format(m.deviceLink.deviceName, { app, system }),
      );
      const entered = await withReauth(() => client.enterDeviceLinkCode(code, name));
      if (entered === undefined) {
        setError(m.deviceLink.notVerified);
        return;
      }
      rememberTyped(entered.id, entered.scan, entered.verifier);
      setTaken(entered.id);
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <Form onSubmit={(event) => void submit(event)} className="flex flex-col gap-3">
      <p className={hintClass}>
        {signedIn ? m.deviceLink.typeHintSignedIn : m.deviceLink.typeHintSignedOut}
      </p>
      <TextField
        name="code"
        isRequired
        isInvalid={error !== null}
        value={value}
        // An invalid field blocks the form's submission, so editing the code clears the error.
        onChange={(next) => {
          setValue(next);
          setError(null);
        }}
        autoComplete="one-time-code"
        className={fieldClass}
      >
        <Label className={labelClass}>{m.deviceLink.typeField}</Label>
        <Input
          autoCapitalize="characters"
          autoCorrect="off"
          spellCheck={false}
          placeholder={m.deviceLink.typePlaceholder}
          className={inputClass + " font-mono tracking-widest uppercase"}
        />
        {error !== null && <p className="text-sm text-danger">{error}</p>}
      </TextField>
      <Button type="submit" isDisabled={pending} className={primaryButtonClass + " self-end"}>
        {m.deviceLink.typeSubmit}
      </Button>
    </Form>
  );
}
