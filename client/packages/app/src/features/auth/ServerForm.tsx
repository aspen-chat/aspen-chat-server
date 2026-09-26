import { normalizeServerUrl } from "@aspen/protocol";
import { useState, type SyntheticEvent } from "react";
import { Button, Form, Input, Label, TextField } from "react-aria-components";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";

/** Asks which Aspen server to connect to. Shown when no server is remembered or configured. */
export function ServerForm({
  initial,
  onSubmit,
}: {
  initial: string;
  onSubmit: (serverUrl: string) => void;
}) {
  const m = useMessages();
  const [error, setError] = useState<string | null>(null);

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
      <h1 className="text-2xl font-semibold">{m.appName}</h1>
      <TextField
        name="server"
        defaultValue={initial}
        isRequired
        isInvalid={error !== null}
        autoComplete="url"
        className="flex flex-col gap-1"
      >
        <Label className="text-sm font-medium text-ink-muted">{m.serverLabel}</Label>
        <Input
          placeholder={m.serverPlaceholder}
          className="rounded-md border border-line bg-surface-raised px-3 py-2 text-base outline-none focus:border-accent focus:ring-2 focus:ring-accent/30 invalid:border-danger"
        />
        {error !== null && <p className="text-sm text-danger">{error}</p>}
      </TextField>
      <Button
        type="submit"
        className="rounded-md bg-accent px-4 py-2 font-medium text-accent-contrast outline-none hover:bg-accent-strong pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        {m.continue}
      </Button>
    </Form>
  );
}
