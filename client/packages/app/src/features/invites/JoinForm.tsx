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
import { parseAspenLink, type AspenLink } from "@/features/qr/aspenLinks";
import { useOpenAspenLink } from "@/features/qr/openLink";
import { inviteLinkExample, parseInvite } from "@/features/invites/inviteCode";
import { routeJoinLink, type OtherServerPrefill } from "@/features/invites/joinRouting";
import { useHomeHosts } from "@/features/messages/knownHosts";
import { openInviteLink, useDomain } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Takes a pasted invite link or code and opens its invite screen: on the deployment the link
 * names or is on, or for a bare code, the deployment shown. On a phone, an invite's QR code can
 * be scanned instead, a community's or a dual invite's. A registration invite of another
 * deployment, scanned or pasted, is handed to `onOtherServer`, which opens the add-a-server form
 * filled in with it (`routeJoinLink`); a place that passes none cannot sign in elsewhere, and
 * the invite is refused, naming its deployment.
 */
export function JoinForm({
  onDone,
  onOtherServer,
}: {
  onDone?: () => void;
  onOtherServer?: ((prefill: OtherServerPrefill) => void) | undefined;
}) {
  const m = useMessages();
  const domain = useDomain();
  const homeHosts = useHomeHosts();
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);
  const open = useOpenAspenLink();

  /** Acts on a link, answering `null`, or what to tell the reader about one it will not take. */
  function take(link: AspenLink): string | null {
    const routing = routeJoinLink(link, onOtherServer !== undefined);
    switch (routing.action) {
      case "open":
        onDone?.();
        open(routing.link);
        return null;
      case "otherServer":
        onOtherServer?.(routing.prefill);
        return null;
      case "refuse":
        return routing.reason === "notAnInvite"
          ? m.qr.notAnInvite
          : format(m.qr.registrationElsewhere, { domain: routing.domain });
    }
  }

  function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const value = new FormData(event.currentTarget).get("invite");
    const text = typeof value === "string" ? value : "";
    const invite = parseInvite(text, homeHosts);
    if (invite !== null) {
      onDone?.();
      void navigate(openInviteLink(invite, domain));
      return;
    }
    // Typed text is an invite to a community unless it is another deployment's registration
    // invite, which is the one other link this field has somewhere to take.
    const link = parseAspenLink(text, homeHosts);
    setError(
      link?.kind === "registration" && link.domain !== null ? take(link) : m.inviteInputInvalid,
    );
  }

  return (
    <Form onSubmit={submit} className="flex w-full flex-col gap-3">
      <TextField
        name="invite"
        isRequired
        isInvalid={error !== null}
        onChange={() => {
          setError(null);
        }}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.inviteInputLabel}</Label>
        <Input className={inputClass} placeholder={inviteLinkExample()} />
        <FieldError className="text-sm text-danger">{error}</FieldError>
      </TextField>
      <Button type="submit" className={primaryButtonClass}>
        {m.continue}
      </Button>
      {detectShell() === "mobile" && (
        <ScanCodeButton
          label={m.qr.scanInvite}
          hint={m.qr.scanInviteHint}
          accept={take}
          className={outlineButtonClass}
        />
      )}
    </Form>
  );
}
