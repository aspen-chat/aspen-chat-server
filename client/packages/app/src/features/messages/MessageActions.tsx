import type { Permission } from "@aspen/protocol";
import {
  ArrowBendUpLeftIcon,
  ChatsCircleIcon,
  CopyIcon,
  EnvelopeSimpleIcon,
  FlagIcon,
  LinkIcon,
  PencilSimpleIcon,
  TrashIcon,
  UsersIcon,
} from "@phosphor-icons/react";
import { useRef, type ReactNode } from "react";
import { useKeepsReadPosition, useReactions, useSync } from "@/api/hooks";
import { copyText } from "@/features/layout/clipboard";
import { CopyIdButton } from "@/features/layout/CopyId";
import { IconAction } from "@/features/layout/IconAction";
import { toast } from "@/features/layout/toast";
import { ReactionPicker, ViewReactionsButton } from "@/features/messages/Reactions";
import { PinButton } from "@/features/messages/PinButton";
import { SaveButton } from "@/features/messages/SaveButton";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { hasComposer, replyIn } from "@/features/messages/composerTargets";
import { markUnread } from "@/features/messages/markUnread";
import { EchoReplyButton } from "@/features/threads/EchoReplyButton";
import { useMessages } from "@/i18n/context";

/** One action's button: square, with its icon, at finger size on a touch screen. */
export const actionClass =
  "rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 pointer-coarse:p-3";

/**
 * One action as a row of a touch screen's list: its icon, then its name, across the sheet's
 * width. Coloured by `listActionClass` or `listDangerClass`.
 */
const listRowClass =
  "flex w-full items-center gap-4 px-4 py-3 text-start outline-none pressed:bg-surface-hover " +
  "focus-visible:bg-surface-hover [&>svg]:shrink-0";
const listActionClass = listRowClass + " text-ink [&>svg]:text-ink-muted";
const listDangerClass = listRowClass + " text-danger";

/** What a message's actions open beyond themselves, each a sheet of its own on a touch screen. */
export type MessageSheet = "react" | "reactions" | "delete" | "report";

/**
 * What can be done to a message: react, reply to it, save it, mark it unread, pin, see who
 * reacted, reply in a thread, also send a thread reply to its channel, edit, copy its text, copy
 * a link to it, report it, copy its id, and delete it. Given `open`, they are a list of icons and
 * names, which every action closes: a touch screen's sheet under a long press
 * (`MessageActionSheet`) and a computer's menu (`MessageContextMenu`); the ones that open
 * something hand that to `open`, since what opens from inside the list would go with it. The
 * list leaves out reacting, which the sheet and the menu offer above it, and ends with deleting,
 * the one that cannot be undone, away from the rest. Without it, they are the row of icon
 * buttons at the message's corner on a computer, while the pointer is over it or focus is in it,
 * with the reaction picker and the reactions dialog opening from their buttons; the row keeps to
 * what is done often, so that it fits a narrow pane, and leaves marking unread, copying, reporting,
 * and deleting to the menu, which its last button opens (`MessageItem`). Each is offered only where the permissions allow it;
 * Reply where its channel's message box is open for the reader to write in (`replyIn`), which is
 * asked as the actions are built, since a row is drawn before the box beside it is.
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
  markableUnread,
  link,
  onOpenThread,
  onEdit,
  open,
  onDone,
  afterClosing,
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
  /**
   * Whether Mark unread is offered: for a message by someone else, since the reader's own
   * messages never make a channel unread to them.
   */
  markableUnread: boolean;
  /** The link to share for it, which Copy link copies. */
  link: string;
  onOpenThread: () => void;
  onEdit: () => void;
  /**
   * Opens a sheet outside the actions. Given (a touch screen's sheet), they are drawn as a
   * list of icons and names, without reacting, and their own picker and dialogs are not used.
   */
  open?: (sheet: MessageSheet) => void;
  /** Called after an action that is done at once, so the sheet offering the actions can close. */
  onDone?: () => void;
  /**
   * Runs an action that puts focus elsewhere once the sheet offering the actions has gone,
   * since the sheet holds focus until then; without it, the action runs at once.
   */
  afterClosing?: (run: () => void) => void;
}) {
  const m = useMessages();
  const hasReactions = useReactions(messageId).size > 0;
  // In a touch screen's sheet: a list of icons and names.
  const labelled = open !== undefined;
  const action = { labelled, className: labelled ? listActionClass : actionClass };
  return (
    <>
      {open === undefined && permissions.has("addReactions") && (
        <ReactionPicker
          messageId={messageId}
          communityId={communityId}
          triggerClassName={actionClass}
          iconSize={ACTION_ICON}
        />
      )}
      {hasComposer(channelId) && (
        <IconAction
          label={m.replies.reply}
          {...action}
          onPress={() => {
            const start = () => {
              replyIn(channelId, messageId);
            };
            if (afterClosing === undefined) {
              start();
            } else {
              afterClosing(start);
            }
            onDone?.();
          }}
          icon={
            <ArrowBendUpLeftIcon
              size={ACTION_ICON}
              aria-hidden="true"
              className="rtl:-scale-x-100"
            />
          }
        />
      )}
      <SaveButton
        messageId={messageId}
        {...action}
        {...(onDone === undefined ? {} : { onPressed: onDone })}
      />
      {labelled && markableUnread && (
        <MarkUnreadButton
          messageId={messageId}
          channelId={channelId}
          {...action}
          {...(onDone === undefined ? {} : { onDone })}
        />
      )}
      {permissions.has("pinMessages") && (
        <PinButton
          messageId={messageId}
          channelId={channelId}
          {...action}
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
          <IconAction
            label={m.viewReactions}
            {...action}
            onPress={() => {
              open("reactions");
            }}
            icon={<UsersIcon size={ACTION_ICON} aria-hidden="true" />}
          />
        )
      )}
      {canThread && (
        <IconAction
          label={m.threads.replyInThread}
          {...action}
          onPress={() => {
            onDone?.();
            onOpenThread();
          }}
          icon={<ChatsCircleIcon size={ACTION_ICON} aria-hidden="true" />}
        />
      )}
      {echoParent !== null && (
        <EchoReplyButton
          messageId={messageId}
          parentId={echoParent}
          {...action}
          {...(onDone === undefined ? {} : { onPressed: onDone })}
        />
      )}
      {editable && (
        <IconAction
          label={m.editMessage}
          {...action}
          onPress={() => {
            onDone?.();
            onEdit();
          }}
          icon={<PencilSimpleIcon size={ACTION_ICON} aria-hidden="true" />}
        />
      )}
      {labelled && text !== null && text !== "" && (
        <CopyButton
          text={text}
          label={m.copyMessageText}
          copied={m.copiedMessageText}
          icon={<CopyIcon size={ACTION_ICON} aria-hidden="true" />}
          {...action}
          {...(onDone === undefined ? {} : { onDone })}
        />
      )}
      {labelled && (
        <CopyButton
          text={link}
          label={m.reports.copyLink}
          copied={m.reports.copiedLink}
          icon={<LinkIcon size={ACTION_ICON} aria-hidden="true" />}
          {...action}
          {...(onDone === undefined ? {} : { onDone })}
        />
      )}
      {labelled && reportable && (
        <IconAction
          label={m.reports.reportMessage}
          {...action}
          onPress={() => {
            open("report");
          }}
          icon={<FlagIcon size={ACTION_ICON} aria-hidden="true" />}
        />
      )}
      {labelled && (
        <CopyIdButton
          id={messageId}
          thing="message"
          {...action}
          iconSize={ACTION_ICON}
          {...(onDone === undefined ? {} : { onCopied: onDone })}
        />
      )}
      {labelled && deletable && (
        <IconAction
          label={m.deleteMessage}
          labelled
          danger
          className={listDangerClass}
          onPress={() => {
            open("delete");
          }}
          icon={<TrashIcon size={ACTION_ICON} aria-hidden="true" />}
        />
      )}
    </>
  );
}

/**
 * Marks the channel unread from the message on, where the reader keeps a read position: a
 * channel or DM they belong to, or a thread.
 */
function MarkUnreadButton({
  messageId,
  channelId,
  labelled,
  className,
  onDone,
}: {
  messageId: string;
  channelId: string;
  labelled: boolean;
  className: string;
  onDone?: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  if (!useKeepsReadPosition(channelId)) {
    return null;
  }
  return (
    <IconAction
      label={m.markUnread}
      labelled={labelled}
      onPress={() => {
        onDone?.();
        markUnread(sync, m, channelId, messageId);
      }}
      className={className}
      icon={<EnvelopeSimpleIcon size={ACTION_ICON} aria-hidden="true" />}
    />
  );
}

/** Copies `text`, and says `copied` in a toast. */
function CopyButton({
  text,
  label,
  copied,
  icon,
  labelled,
  className,
  onDone,
}: {
  text: string;
  label: string;
  copied: string;
  icon: ReactNode;
  labelled: boolean;
  className: string;
  onDone?: () => void;
}) {
  const button = useRef<HTMLButtonElement>(null);
  return (
    <IconAction
      ref={button}
      label={label}
      labelled={labelled}
      onPress={() => {
        // A menu's item has no element of its own; the menu has closed by now.
        void copyText(text, button.current ?? document.body).then((ok) => {
          if (ok) {
            toast(copied);
            onDone?.();
          }
        });
      }}
      className={className}
      icon={icon}
    />
  );
}
