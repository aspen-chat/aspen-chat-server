import type { Permission } from "@aspen/protocol";
import {
  ChatsCircleIcon,
  CopyIcon,
  FlagIcon,
  LinkIcon,
  PencilSimpleIcon,
  SmileyIcon,
  TrashIcon,
  UsersIcon,
} from "@phosphor-icons/react";
import { useRef, type ReactNode } from "react";
import { Button } from "react-aria-components";
import { useReactions } from "@/api/hooks";
import { copyText } from "@/features/layout/clipboard";
import { CopyIdButton } from "@/features/layout/CopyId";
import { toast } from "@/features/layout/toast";
import { Tooltip } from "@/features/layout/Tooltip";
import { DeleteMessageDialog } from "@/features/messages/DeleteMessageDialog";
import { ReactionPicker, ViewReactionsButton } from "@/features/messages/Reactions";
import { PinButton } from "@/features/messages/PinButton";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { ReportMessageButton } from "@/features/reports/ReportDialog";
import { EchoReplyButton } from "@/features/threads/EchoReplyButton";
import { useMessages } from "@/i18n/context";

/** One action's button: square, with its icon, at finger size on a touch screen. */
export const actionClass =
  "rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 pointer-coarse:p-3";

/** What a message's actions open beyond themselves, each a sheet of its own on a touch screen. */
export type MessageSheet = "react" | "reactions" | "delete" | "report";

/**
 * What can be done to a message, as a row of icon buttons: react, pin, see who reacted, reply
 * in a thread, also send a thread reply to its channel, edit, delete, copy its text, copy a link to it, report it, and copy its id. A computer shows the row at the
 * message's corner while the pointer is over it or focus is in it, and the picker and the
 * dialogs open from their buttons. A touch screen shows it in a popover under a long press
 * (`MessageItem`), which every action closes: the ones that open something hand that to
 * `open`, since what opens from inside the popover would go with it. Each is offered only
 * where the permissions allow it.
 */
export function MessageActions({
  messageId,
  channelId,
  communityId,
  text,
  permissions,
  canThread,
  echoParent,
  editable,
  deletable,
  reportable,
  link,
  onOpenThread,
  onEdit,
  open,
  onDone,
}: {
  messageId: string;
  channelId: string;
  /** The community the message is in, whose own emoji reactions may be; none in a DM. */
  communityId: string | null;
  /** The message's text, which Copy text copies; none for a message without any. */
  text: string | null;
  permissions: ReadonlySet<Permission>;
  canThread: boolean;
  /** For a thread reply its author may still echo, the thread's parent channel; else null. */
  echoParent: string | null;
  editable: boolean;
  deletable: boolean;
  /** Whether the reader may report it to the moderators: not their own, nor a notice. */
  reportable: boolean;
  /** The link to share for it, which Copy link copies. */
  link: string;
  onOpenThread: () => void;
  onEdit: () => void;
  /** Opens a sheet outside the row; given, the row's own picker and dialogs are not used. */
  open?: (sheet: MessageSheet) => void;
  /** Called after an action that is done at once, so a popover offering the row can close. */
  onDone?: () => void;
}) {
  const m = useMessages();
  const hasReactions = useReactions(messageId).size > 0;
  return (
    <>
      {permissions.has("addReactions") &&
        (open === undefined ? (
          <ReactionPicker
            messageId={messageId}
            communityId={communityId}
            triggerClassName={actionClass}
            iconSize={ACTION_ICON}
          />
        ) : (
          <Action
            label={m.addReaction}
            onPress={() => {
              open("react");
            }}
          >
            <SmileyIcon size={ACTION_ICON} aria-hidden="true" />
          </Action>
        ))}
      {permissions.has("pinMessages") && (
        <PinButton
          messageId={messageId}
          channelId={channelId}
          className={actionClass}
          {...(onDone === undefined ? {} : { onPressed: onDone })}
        />
      )}
      {open === undefined ? (
        <ViewReactionsButton
          messageId={messageId}
          communityId={communityId}
          triggerClassName={actionClass}
        />
      ) : (
        hasReactions && (
          <Action
            label={m.viewReactions}
            onPress={() => {
              open("reactions");
            }}
          >
            <UsersIcon size={ACTION_ICON} aria-hidden="true" />
          </Action>
        )
      )}
      {canThread && (
        <Action
          label={m.threads.replyInThread}
          onPress={() => {
            onDone?.();
            onOpenThread();
          }}
        >
          <ChatsCircleIcon size={ACTION_ICON} aria-hidden="true" />
        </Action>
      )}
      {echoParent !== null && (
        <EchoReplyButton
          messageId={messageId}
          parentId={echoParent}
          className={actionClass}
          {...(onDone === undefined ? {} : { onPressed: onDone })}
        />
      )}
      {editable && (
        <Action
          label={m.editMessage}
          onPress={() => {
            onDone?.();
            onEdit();
          }}
        >
          <PencilSimpleIcon size={ACTION_ICON} aria-hidden="true" />
        </Action>
      )}
      {deletable &&
        (open === undefined ? (
          <DeleteMessageDialog
            messageId={messageId}
            triggerClassName={actionClass + " text-danger"}
          />
        ) : (
          <Action
            label={m.deleteMessage}
            className={actionClass + " text-danger"}
            onPress={() => {
              open("delete");
            }}
          >
            <TrashIcon size={ACTION_ICON} aria-hidden="true" />
          </Action>
        ))}
      {text !== null && text !== "" && (
        <CopyTextButton text={text} {...(onDone === undefined ? {} : { onDone })} />
      )}
      <CopyButton
        text={link}
        label={m.reports.copyLink}
        copied={m.reports.copiedLink}
        icon={<LinkIcon size={ACTION_ICON} aria-hidden="true" />}
        {...(onDone === undefined ? {} : { onDone })}
      />
      {reportable &&
        (open === undefined ? (
          <ReportMessageButton messageId={messageId} triggerClassName={actionClass} />
        ) : (
          <Action
            label={m.reports.reportMessage}
            onPress={() => {
              open("report");
            }}
          >
            <FlagIcon size={ACTION_ICON} aria-hidden="true" />
          </Action>
        ))}
      <CopyIdButton
        id={messageId}
        thing="message"
        className={actionClass}
        {...(onDone === undefined ? {} : { onCopied: onDone })}
      />
    </>
  );
}

/** One action: an icon button named by its tooltip. */
function Action({
  label,
  onPress,
  className = actionClass,
  children,
}: {
  label: string;
  onPress: () => void;
  className?: string;
  children: ReactNode;
}) {
  return (
    <Tooltip text={label}>
      <Button onPress={onPress} aria-label={label} className={className}>
        {children}
      </Button>
    </Tooltip>
  );
}

/** Copies the message's text, and says so in a toast. */
function CopyTextButton({ text, onDone }: { text: string; onDone?: () => void }) {
  const m = useMessages();
  return (
    <CopyButton
      text={text}
      label={m.copyMessageText}
      copied={m.copiedMessageText}
      icon={<CopyIcon size={ACTION_ICON} aria-hidden="true" />}
      {...(onDone === undefined ? {} : { onDone })}
    />
  );
}

/** Copies `text`, and says `copied` in a toast. */
function CopyButton({
  text,
  label,
  copied,
  icon,
  onDone,
}: {
  text: string;
  label: string;
  copied: string;
  icon: ReactNode;
  onDone?: () => void;
}) {
  const button = useRef<HTMLButtonElement>(null);
  return (
    <Tooltip text={label}>
      <Button
        ref={button}
        aria-label={label}
        onPress={() => {
          if (button.current !== null) {
            void copyText(text, button.current).then((ok) => {
              if (ok) {
                toast(copied);
                onDone?.();
              }
            });
          }
        }}
        className={actionClass}
      >
        {icon}
      </Button>
    </Tooltip>
  );
}
