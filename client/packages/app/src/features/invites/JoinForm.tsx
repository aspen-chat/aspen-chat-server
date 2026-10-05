import { useNavigate } from "@tanstack/react-router";
import { useState, type SyntheticEvent } from "react";
import { Button, FieldError, Form, Input, Label, TextField } from "react-aria-components";
import {
  fieldClass,
  inputClass,
  labelClass,
  outlineButtonClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { detectShell } from "@/config";
import { ScanCodeButton } from "@/features/qr/ScanCode";
import { useOpenAspenLink } from "@/features/qr/openLink";
import { inviteLinkExample, parseInvite } from "@/features/invites/inviteCode";
import { openInviteLink, useDomain } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";

/**
 * Takes a pasted invite link or code and opens its invite screen: on the deployment the link
 * names, or for a bare code, the deployment shown. On a phone, an invite's QR code can be
 * scanned instead, a community's or a dual invite's.
 */
export function JoinForm({ onDone }: { onDone?: () => void }) {
  const m = useMessages();
  const domain = useDomain();
  const navigate = useNavigate();
  const [invalid, setInvalid] = useState(false);
  const open = useOpenAspenLink();

  function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const value = new FormData(event.currentTarget).get("invite");
    const invite = parseInvite(typeof value === "string" ? value : "");
    if (invite === null) {
      setInvalid(true);
      return;
    }
    onDone?.();
    void navigate(openInviteLink(invite, domain));
  }

  return (
    <Form onSubmit={submit} className="flex w-full flex-col gap-3">
      <TextField
        name="invite"
        isRequired
        isInvalid={invalid}
        onChange={() => {
          setInvalid(false);
        }}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.inviteInputLabel}</Label>
        <Input className={inputClass} placeholder={inviteLinkExample()} />
        <FieldError className="text-sm text-danger">
          {invalid ? m.inviteInputInvalid : null}
        </FieldError>
      </TextField>
      <Button type="submit" className={primaryButtonClass}>
        {m.continue}
      </Button>
      {detectShell() === "mobile" && (
        <ScanCodeButton
          label={m.qr.scanInvite}
          hint={m.qr.scanInviteHint}
          accept={(link) => {
            if (link.kind !== "invite" && link.kind !== "registration") {
              return m.qr.notAnInvite;
            }
            onDone?.();
            open(link);
            return null;
          }}
          className={outlineButtonClass}
        />
      )}
    </Form>
  );
}
