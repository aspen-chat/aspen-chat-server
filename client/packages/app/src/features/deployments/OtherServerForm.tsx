import { ApiProblemError } from "@aspen/protocol";
import { useNavigate } from "@tanstack/react-router";
import { useState, type SyntheticEvent } from "react";
import { Button, Form, Input, Label, Text, TextField } from "react-aria-components";
import { useDeploymentsHub } from "@/api/deploymentsContext";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { parseInvite } from "@/features/invites/inviteCode";
import { deploymentLink, inviteLink } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Signs in at another deployment with the user's home account: its domain, and optionally an
 * invite to one of its communities, which opens once they are in. A deployment that asks a
 * first arrival for a registration invite says so, and the form then asks for one.
 */
export function OtherServerForm({ onDone }: { onDone?: () => void }) {
  const m = useMessages();
  const hub = useDeploymentsHub();
  const navigate = useNavigate();
  const [domain, setDomain] = useState("");
  const [communityInvite, setCommunityInvite] = useState("");
  const [registrationInvite, setRegistrationInvite] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const target = domain.trim().toLowerCase();
    const invite = communityInvite.trim() === "" ? null : parseInvite(communityInvite);
    if (communityInvite.trim() !== "" && invite === null) {
      setError(m.inviteInputInvalid);
      return;
    }
    if (invite?.domain != null && invite.domain !== target) {
      setError(format(m.deployments.inviteElsewhere, { domain: invite.domain, target }));
      return;
    }
    setPending(true);
    setError(null);
    try {
      const registration = registrationInvite?.trim();
      await hub.join(target, registration === "" ? undefined : registration);
      onDone?.();
      await navigate(invite === null ? deploymentLink(target) : inviteLink(target, invite.code));
    } catch (e) {
      if (e instanceof ApiProblemError && e.problem.code === "registrationInviteRequired") {
        setRegistrationInvite((current) => current ?? "");
      }
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <Form
      onSubmit={(event) => {
        void submit(event);
      }}
      className="flex w-full flex-col gap-3"
    >
      <TextField value={domain} onChange={setDomain} isRequired className={fieldClass}>
        <Label className={labelClass}>{m.deployments.domain}</Label>
        <Input
          placeholder={m.deployments.domainPlaceholder}
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          className={inputClass}
        />
      </TextField>
      <TextField
        value={communityInvite}
        onChange={(value) => {
          setCommunityInvite(value);
          // A link names its deployment, which fills in the domain when none is given.
          const named = parseInvite(value)?.domain;
          if (named != null && domain.trim() === "") {
            setDomain(named);
          }
        }}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.deployments.communityInvite}</Label>
        <Input className={inputClass} />
        <Text slot="description" className={hintClass}>
          {m.deployments.communityInviteHint}
        </Text>
      </TextField>
      {registrationInvite !== null && (
        <TextField
          value={registrationInvite}
          onChange={setRegistrationInvite}
          isRequired
          className={fieldClass}
        >
          <Label className={labelClass}>{m.deployments.registrationInvite}</Label>
          <Input className={inputClass} />
        </TextField>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
        {pending ? m.deployments.signingInShort : m.deployments.signIn}
      </Button>
    </Form>
  );
}
