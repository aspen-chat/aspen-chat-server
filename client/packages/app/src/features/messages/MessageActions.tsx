import type { Permission } from "@aspen/protocol";
import {
  ChatsCircleIcon,
  CopyIcon,
  FlagIcon,
  LinkIcon,
  PencilSimpleIcon,
  TrashIcon,
  UsersIcon,
} from "@phosphor-icons/react";
import { useRef, type ReactNode } from "react";
import { useReactions } from "@/api/hooks";
import { copyText } from "@/features/layout/clipboard";
import { CopyIdButton } from "@/features/layout/CopyId";
import { IconAction } from "@/features/layout/IconAction";
import { toast } from "@/features/layout/toast";
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
 * What can be done to a message: react, pin, see who reacted, reply in a thread, also send a
 * thread reply to its channel, edit, delete, copy its text, copy a link to it, report it, and
 * copy its id. A computer shows them as a row of icon buttons at the message's corner while
 * the pointer is over it or focus is in it, and the picker and the dialogs open from their
 * buttons. A touch screen shows them as a list of icons and names, in a sheet under a
 * long press (`MessageActionSheet`), which every action closes: the ones that open something
 * hand that to `open`, since what opens from inside the sheet would go with it. The list
 * leaves out reacting, which the sheet offers above it. Each is offered only where the
 * permissions allow it.
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
  /**
   * Opens a sheet outside the actions. Given (a touch screen's sheet), they are drawn as a
   * list of icons and names, without reacting, and their own picker and dialogs are not used.
   */
  open?: (sheet: MessageSheet) => void;
  /** Called after an action that is done at once, so the sheet offering the actions can close. */
  onDone?: () => void;
}) {
  const m = useMessages();
  const hasReactions = useReactions(messageId).size > 0;
  // In a touch screen's sheet: a list of icons and names.
  const labelled = open !== undefined;
  const action = { labelled, className: labelled ? listActionClass : actionClass };
  const dangerClass = labelled ? listDangerClass : actionClass + " text-danger";
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
      {deletable &&
        (open === undefined ? (
          <DeleteMessageDialog messageId={messageId} triggerClassName={dangerClass} />
        ) : (
          <IconAction
            label={m.deleteMessage}
            labelled
            className={dangerClass}
            onPress={() => {
              open("delete");
            }}
            icon={<TrashIcon size={ACTION_ICON} aria-hidden="true" />}
          />
        ))}
      {text !== null && text !== "" && (
        <CopyButton
          text={text}
          label={m.copyMessageText}
          copied={m.copiedMessageText}
          icon={<CopyIcon size={ACTION_ICON} aria-hidden="true" />}
          {...action}
          {...(onDone === undefined ? {} : { onDone })}
        />
      )}
      <CopyButton
        text={link}
        label={m.reports.copyLink}
        copied={m.reports.copiedLink}
        icon={<LinkIcon size={ACTION_ICON} aria-hidden="true" />}
        {...action}
        {...(onDone === undefined ? {} : { onDone })}
      />
      {reportable &&
        (open === undefined ? (
          <ReportMessageButton messageId={messageId} triggerClassName={actionClass} />
        ) : (
          <IconAction
            label={m.reports.reportMessage}
            {...action}
            onPress={() => {
              open("report");
            }}
            icon={<FlagIcon size={ACTION_ICON} aria-hidden="true" />}
          />
        ))}
      <CopyIdButton
        id={messageId}
        thing="message"
        {...action}
        iconSize={ACTION_ICON}
        {...(onDone === undefined ? {} : { onCopied: onDone })}
      />
    </>
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
        if (button.current !== null) {
          void copyText(text, button.current).then((ok) => {
            if (ok) {
              toast(copied);
              onDone?.();
            }
          });
        }
      }}
      className={className}
      icon={icon}
    />
  );
}
