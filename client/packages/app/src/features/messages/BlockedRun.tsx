import { ProhibitIcon } from "@phosphor-icons/react";
import { Fragment, useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import { useRowProps } from "@/features/messages/messageRows";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Messages by people the reader blocked, collapsed into a row that says how many and opens them
 * on request. The "New Messages" line falls after the row while it is closed, and at
 * `lineOffset` among the messages while it is open.
 */
export function BlockedRun({
  ids,
  lineOffset,
  highlightId,
  item,
}: {
  ids: readonly string[];
  lineOffset: number | null;
  highlightId: string | undefined;
  item: (id: string) => ReactNode;
}) {
  const m = useMessages();
  const [open, setOpen] = useState(highlightId !== undefined && ids.includes(highlightId));
  // A link to one of these messages, followed while the row is shown, opens it.
  const [linked, setLinked] = useState(highlightId);
  // Closed, the row stands for its messages among the list's rows, under the newest one's id.
  const rowProps = useRowProps(ids[ids.length - 1] ?? "");
  if (highlightId !== linked) {
    setLinked(highlightId);
    if (highlightId !== undefined && ids.includes(highlightId)) {
      setOpen(true);
    }
  }
  const count =
    ids.length === 1
      ? m.blocking.oneBlockedMessage
      : format(m.blocking.blockedMessages, { count: String(ids.length) });
  const toggle = (
    <Button
      onPress={() => {
        setOpen(!open);
      }}
      aria-expanded={open}
      className="tap-target rounded font-medium text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      {open ? m.blocking.hide : m.blocking.show}
    </Button>
  );
  if (!open) {
    return (
      <>
        <div
          data-message-id={ids[ids.length - 1]}
          {...rowProps}
          className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-ink-faint outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <ProhibitIcon size={16} aria-hidden="true" />
          <span>{count}</span>
          {toggle}
        </div>
        {lineOffset !== null && <NewMessagesLine />}
      </>
    );
  }
  return (
    <div className="flex flex-col gap-1 border-s-2 border-line ps-2">
      <div className="flex items-center gap-2 px-2 py-1 text-sm text-ink-faint">
        <ProhibitIcon size={16} aria-hidden="true" />
        <span>{count}</span>
        {toggle}
      </div>
      {ids.map((id, index) => (
        <Fragment key={id}>
          {item(id)}
          {index === lineOffset && <NewMessagesLine />}
        </Fragment>
      ))}
    </div>
  );
}

/** The accent line under the last message read, with "New Messages" at its centre. */
export function NewMessagesLine() {
  const m = useMessages();
  return (
    <div
      role="separator"
      aria-label={m.newMessages}
      className="flex items-center gap-2 py-1 text-xs font-semibold text-accent"
    >
      <span aria-hidden="true" className="h-px flex-1 bg-accent" />
      <span aria-hidden="true">{m.newMessages}</span>
      <span aria-hidden="true" className="h-px flex-1 bg-accent" />
    </div>
  );
}
