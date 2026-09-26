import { ApiProblemError } from "@aspen/protocol";
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

/** Names a new category and adds it at the end of the community's list. */
export function CreateCategoryForm({
  communityId,
  onDone,
}: {
  communityId: string;
  onDone: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
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
      await sync.createCategory(communityId, name);
      onDone();
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
      className="flex flex-col gap-3"
    >
      <TextField name="name" isRequired autoFocus className={fieldClass}>
        <Label className={labelClass}>{m.categoryNameLabel}</Label>
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
