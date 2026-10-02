import type { Permission } from "@aspen/protocol";
import { ChatsCircleIcon, CheckIcon, CopyIcon, PencilSimpleIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { Button } from "react-aria-components";
import { copyText } from "@/features/layout/clipboard";
import { CopyIdButton } from "@/features/layout/CopyId";
import { Tooltip } from "@/features/layout/Tooltip";
import { DeleteMessageDialog } from "@/features/messages/DeleteMessageDialog";
import { ReactionPicker, ViewReactionsButton } from "@/features/messages/Reactions";
import { PinButton } from "@/features/messages/PinButton";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { useMessages } from "@/i18n/context";

/** How long the copy button says the text was copied. */
const COPIED_MS = 1500;
/** One action's button: square, with its icon, at finger size on a touch screen. */
export const actionClass =
  "rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 pointer-coarse:p-3";

/**
 * What can be done to a message, as a row of icon buttons: react, pin, see who reacted, reply
 * in a thread, edit, delete, copy its text, and copy its id. A computer shows the row at the
 * message's corner while the pointer is over it or focus is in it; a touch screen shows it in
 * a popover under the message that a long press opens (`MessageItem`). Each is offered only
 * where the permissions allow it.
 */
export function MessageActions({
  messageId,
  channelId,
  text,
  permissions,
  canThread,
  editable,
  deletable,
  onOpenThread,
  onEdit,
  onDone,
}: {
  messageId: string;
  channelId: string;
  /** The message's text, which Copy text copies; none for a message without any. */
  text: string | null;
  permissions: ReadonlySet<Permission>;
  canThread: boolean;
  editable: boolean;
  deletable: boolean;
  onOpenThread: () => void;
  onEdit: () => void;
  /** Called after an action that is done at once, so a popover offering the row can close. */
  onDone?: () => void;
}) {
  const m = useMessages();
  return (
    <>
      {permissions.has("addReactions") && (
        <ReactionPicker
          messageId={messageId}
          triggerClassName={actionClass}
          iconSize={ACTION_ICON}
        />
      )}
      {permissions.has("pinMessages") && (
        <PinButton messageId={messageId} channelId={channelId} className={actionClass} />
      )}
      <ViewReactionsButton messageId={messageId} triggerClassName={actionClass} />
      {canThread && (
        <Tooltip text={m.threads.replyInThread}>
          <Button
            onPress={() => {
              onDone?.();
              onOpenThread();
            }}
            aria-label={m.threads.replyInThread}
            className={actionClass}
          >
            <ChatsCircleIcon size={ACTION_ICON} aria-hidden="true" />
          </Button>
        </Tooltip>
      )}
      {editable && (
        <Tooltip text={m.editMessage}>
          <Button
            onPress={() => {
              onDone?.();
              onEdit();
            }}
            aria-label={m.editMessage}
            className={actionClass}
          >
            <PencilSimpleIcon size={ACTION_ICON} aria-hidden="true" />
          </Button>
        </Tooltip>
      )}
      {deletable && (
        <DeleteMessageDialog
          messageId={messageId}
          triggerClassName={actionClass + " text-danger"}
        />
      )}
      {text !== null && text !== "" && <CopyTextButton text={text} />}
      <CopyIdButton id={messageId} thing="message" className={actionClass} />
    </>
  );
}

/** Copies the message's text, and says so for a moment. */
function CopyTextButton({ text }: { text: string }) {
  const m = useMessages();
  const button = useRef<HTMLButtonElement>(null);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) {
      return;
    }
    const timer = setTimeout(() => {
      setCopied(false);
    }, COPIED_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [copied]);
  const label = copied ? m.copiedMessageText : m.copyMessageText;
  return (
    <Tooltip text={label}>
      <Button
        ref={button}
        aria-label={label}
        onPress={() => {
          if (button.current !== null) {
            void copyText(text, button.current).then(setCopied);
          }
        }}
        className={actionClass}
      >
        {copied ? (
          <CheckIcon size={ACTION_ICON} aria-hidden="true" />
        ) : (
          <CopyIcon size={ACTION_ICON} aria-hidden="true" />
        )}
      </Button>
    </Tooltip>
  );
}
