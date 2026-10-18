import type { Message } from "@aspen/protocol";
import { ArrowBendDownRightIcon, CopyIcon, LinkIcon } from "@phosphor-icons/react";
import { Link, useNavigate, type LinkComponentProps } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Menu } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { useBlocked } from "@/api/hooks";
import { ActionsAsMenuItems } from "@/features/layout/actionsAsMenuItems";
import { copyText } from "@/features/layout/clipboard";
import { ContextMenu } from "@/features/layout/ContextMenu";
import { useContextMenu, type ContextMenuState } from "@/features/layout/contextMenuState";
import { MenuAction, menuClass } from "@/features/layout/menu";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { toast } from "@/features/layout/toast";
import { messageUrl, type ChannelHome } from "@/features/messages/links";
import { MessageBody } from "@/features/messages/MessageBody";
import { PersonAvatar, PersonName } from "@/features/users/PersonName";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/** An icon button among a listed message's `actions`. */
export const listedActionClass =
  "tap-target rounded p-0.5 text-ink-muted outline-none hover:bg-surface-raised hover:text-ink " +
  "pressed:bg-surface-raised focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * One message listed out of its channel (a channel's pins, saved messages, the activity feed):
 * its author's picture and name, when it was sent, where (`where`, when the list spans
 * channels), a way to go to it (`link`), and the message drawn as the channel draws it
 * (`MessageBody`: its text, tags, pictures, files, poll, and link cards). `actions` stand beside
 * the way to it, `mark` before the name, and `children` below the message. A message by someone
 * the reader blocked is not quoted; going to it opens it. `undefined` while it is on its way.
 * Its text is the size of the list it stands in, which sets one: `text-sm`, or `message-text`
 * to follow the reader's message text size. Right-clicking it offers the same by name
 * (`ListedMessageMenu`).
 */
export function ListedMessage({
  message,
  home,
  link,
  onJump,
  where,
  mark,
  actions,
  children,
}: {
  message: Message | undefined;
  home: ChannelHome;
  link: LinkComponentProps | undefined;
  onJump?: () => void;
  where?: string;
  mark?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  const m = useMessages();
  const blocked = useBlocked(message?.author);
  const time = useDateFormat(TIME);
  const menu = useContextMenu();
  return (
    <li
      onContextMenu={(event) => {
        if (message !== undefined) {
          menu.onContextMenu(event);
        }
      }}
      className="flex gap-2 rounded-md px-2 py-1.5 hover:bg-surface-hover"
    >
      {menu.opened && message !== undefined && (
        <ListedMessageMenu
          menu={menu}
          message={message}
          home={home}
          link={link}
          {...(onJump === undefined ? {} : { onJump })}
          actions={actions}
        />
      )}
      <PersonAvatar id={message?.author} size="sm" />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex items-baseline gap-2">
          {mark}
          <span className="truncate font-medium">
            <PersonName id={message?.author} community={home.community} />
          </span>
          {message !== undefined && (
            <span className="shrink-0 text-xs text-ink-faint">
              {time.format(new Date(message.timestamp))}
            </span>
          )}
          <span className="ms-auto flex shrink-0 items-center gap-1">
            {link !== undefined && (
              <Link
                {...link}
                {...(onJump === undefined ? {} : { onClick: onJump })}
                className="rounded px-1 text-xs text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
              >
                {m.pins.jump}
              </Link>
            )}
            {actions}
          </span>
        </div>
        {where !== undefined && where !== "" && (
          <span className="text-xs text-ink-muted">{where}</span>
        )}
        {blocked ? (
          <span className="text-ink-faint italic">
            {format(m.blocking.blockedMessages, { count: 1 })}
          </span>
        ) : message === undefined ? (
          <>
            <LoadingLabel />
            <Skeleton className="h-3.5 w-11/12" />
            <Skeleton className="h-3.5 w-1/2" />
          </>
        ) : (
          <div className="flex min-w-0 flex-col gap-1 break-words">
            <MessageBody message={message} home={home} />
          </div>
        )}
        {children}
      </div>
    </li>
  );
}

/**
 * A listed message's menu: going to it, its row's `actions` by name (each an `IconAction`), and
 * copying its text and the link to it.
 */
function ListedMessageMenu({
  menu,
  message,
  home,
  link,
  onJump,
  actions,
}: {
  menu: ContextMenuState;
  message: Message;
  home: ChannelHome;
  link: LinkComponentProps | undefined;
  onJump?: () => void;
  actions: ReactNode;
}) {
  const m = useMessages();
  const navigate = useNavigate();
  const client = useAspenClient();
  const copy = (text: string, copied: string) => {
    void copyText(text, document.body).then((ok) => {
      if (ok) {
        toast(copied);
      }
    });
  };
  return (
    <ContextMenu menu={menu} label={m.messageActionsLabel}>
      <Menu
        aria-label={m.messageActionsLabel}
        autoFocus={menu.byKeyboard ? "first" : false}
        onAction={() => {
          menu.onOpenChange(false);
        }}
        className={menuClass}
      >
        {link !== undefined && (
          <MenuAction
            label={m.pins.jump}
            icon={<ArrowBendDownRightIcon aria-hidden="true" className="rtl:-scale-x-100" />}
            onAction={() => {
              onJump?.();
              void navigate(link);
            }}
          />
        )}
        <ActionsAsMenuItems value>{actions}</ActionsAsMenuItems>
        {message.content !== "" && (
          <MenuAction
            label={m.copyMessageText}
            icon={<CopyIcon aria-hidden="true" />}
            onAction={() => {
              copy(message.content, m.copiedMessageText);
            }}
          />
        )}
        <MenuAction
          label={m.reports.copyLink}
          icon={<LinkIcon aria-hidden="true" />}
          onAction={() => {
            copy(
              messageUrl(client.baseUrl, home.community, message.channelId, message.id),
              m.reports.copiedLink,
            );
          }}
        />
      </Menu>
    </ContextMenu>
  );
}

/** A listed message on its way, shaped like `ListedMessage`: a picture, a name and time, and two lines. */
export function ListedMessageSkeleton({ index }: { index: number }) {
  return (
    <div className="flex gap-2 px-2 py-1.5">
      <Skeleton className="h-6 w-6 shrink-0 rounded-full" />
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <Skeleton className={"h-3.5 " + (index % 2 === 0 ? "w-24" : "w-16")} />
        <Skeleton className="h-3.5 w-11/12" />
        <Skeleton className={"h-3.5 " + (index === 1 ? "w-3/4" : "w-1/2")} />
      </div>
    </div>
  );
}
