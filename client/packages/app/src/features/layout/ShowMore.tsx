import { ApiProblemError } from "@aspen/protocol";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useListComplete } from "@/api/hooks";
import { alertClass } from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";

/**
 * The button under a paged list (`topic`, as the store names it) that reads its next page,
 * shown while the server has one.
 */
export function ShowMore({ topic, load }: { topic: string; load: () => Promise<void> }) {
  const m = useMessages();
  const complete = useListComplete(topic);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (complete) {
    return null;
  }

  async function more() {
    setBusy(true);
    setError(null);
    try {
      await load();
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col items-start gap-1">
      <Button
        className={secondaryButtonClass}
        isDisabled={busy}
        onPress={() => {
          void more();
        }}
      >
        {busy ? m.loading : m.showMore}
      </Button>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
    </div>
  );
}
