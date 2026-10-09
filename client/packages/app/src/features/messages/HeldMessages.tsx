import type { HeldEntry } from "@aspen/protocol";
import { ApiProblemError } from "@aspen/protocol";
import { CircleNotchIcon, WarningCircleIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useHeldMessages, useSync } from "@/api/hooks";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The caller's messages that the server holds while a preview of one of their files is made,
 * above the message box of `place` (`heldPlace`: a channel, or a thread not made yet): each
 * waits, said so, until it is posted (and appears in the list) or dropped, when it says why and
 * offers to send it again or let it go.
 */
export function HeldMessages({ place }: { place: string }) {
  const m = useMessages();
  const held = useHeldMessages(place);
  if (held.length === 0) {
    return null;
  }
  return (
    <ul aria-label={m.held.label} className="flex flex-col gap-1">
      {held.map((entry) => (
        <HeldRow key={entry.message.id} entry={entry} />
      ))}
    </ul>
  );
}

function HeldRow({ entry }: { entry: HeldEntry }) {
  const m = useMessages();
  const sync = useSync();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { message, failure } = entry;
  const count = message.attachments.length;
  const files = count === 1 ? m.held.oneFile : format(m.held.files, { count: String(count) });
  return (
    <li
      className="motion-rise flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-line bg-surface-sunken px-3 py-2 text-sm"
      data-held-message={message.id}
    >
      {failure === null ? (
        <CircleNotchIcon
          size={16}
          aria-hidden="true"
          className="shrink-0 animate-spin text-ink-muted motion-reduce:animate-none"
        />
      ) : (
        <WarningCircleIcon size={16} aria-hidden="true" className="shrink-0 text-danger" />
      )}
      <span className="min-w-0 flex-1 truncate text-ink-muted">
        {message.content === "" ? files : `${message.content} · ${files}`}
      </span>
      {failure === null ? (
        <span role="status" className="text-xs text-ink-faint">
          {m.held.waiting}
        </span>
      ) : (
        <>
          <span role="alert" className="w-full text-xs text-danger">
            {format(m.held.failed, { reason: error ?? failure })}
          </span>
          <Button
            isDisabled={pending}
            onPress={() => {
              setPending(true);
              setError(null);
              sync
                .sendHeldAgain(message.id)
                .catch((e: unknown) => {
                  setError(e instanceof ApiProblemError ? e.message : String(e));
                })
                .finally(() => {
                  setPending(false);
                });
            }}
            className={secondaryButtonClass}
          >
            {m.held.sendAgain}
          </Button>
          <Button
            isDisabled={pending}
            onPress={() => {
              sync.store.forgetHeldMessage(message.id);
            }}
            className={secondaryButtonClass}
          >
            {m.held.discard}
          </Button>
        </>
      )}
    </li>
  );
}
