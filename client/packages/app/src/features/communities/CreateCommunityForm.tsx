import { ApiProblemError } from "@aspen/protocol";
import { useNavigate } from "@tanstack/react-router";
import { useState, type SyntheticEvent } from "react";
import { Button, FieldError, Form, Input, Label, TextField } from "react-aria-components";
import { useSync } from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";

/** Names a new community, creates it, and opens it. */
export function CreateCommunityForm({ onDone }: { onDone?: () => void }) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = formString(new FormData(event.currentTarget), "name").trim();
    if (name.length === 0) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      const community = await sync.createCommunity(name);
      onDone?.();
      await navigate({ to: "/communities/$communityId", params: { communityId: community.id } });
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <Form
      onSubmit={(e) => {
        void submit(e);
      }}
      className="flex w-full flex-col gap-3"
    >
      <TextField name="name" isRequired autoFocus className={fieldClass}>
        <Label className={labelClass}>{m.communityNameLabel}</Label>
        <Input className={inputClass} />
        <FieldError className="text-sm text-danger" />
      </TextField>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
        {pending ? m.creating : m.create}
      </Button>
    </Form>
  );
}
