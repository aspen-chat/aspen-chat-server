import type { Channel } from "@aspen/protocol";
import { ArrowSquareOutIcon, CaretRightIcon, CopyIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { Fragment, useContext, useRef, useState, type ReactNode } from "react";
import { Dialog, Menu, MenuItem, Popover } from "react-aria-components";
import { HomeClientContext } from "@/api/context";
import { ScopeDomainContext } from "@/api/deploymentsContext";
import { SourceScope } from "@/api/deployments";
import { useSources } from "@/api/everywhere";
import {
  useChannel,
  useChannelLoading,
  useChannelOnDemand,
  useCommunity,
  useLinkedMessage,
  useMessage,
} from "@/api/hooks";
import { adminTabLabel, isAdminTab } from "@/features/admin/adminTabs";
import { useDmTitle } from "@/features/dms/useDmTitle";
import { copyText } from "@/features/layout/clipboard";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { toast } from "@/features/layout/toast";
import { feelPress } from "@/features/messages/haptics";
import { selfLinkRoute, type SelfLinkTarget, type SelfRoute } from "@/features/messages/selfLinks";
import { PersonName } from "@/features/users/PersonName";
import { useMessages } from "@/i18n/context";
import { formatNodes } from "@/i18n/formatNodes";
import { format, type Messages } from "@/i18n/messages";

/** Inline, so its names wrap with the sentence around it and read as one run of text. */
const chipClass =
  "rounded bg-accent-soft px-1 font-medium text-accent-strong outline-none box-decoration-clone " +
  "select-none [-webkit-touch-callout:none] hover:underline " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";
const popoverClass = "w-56 rounded-md border border-line bg-surface-raised p-1 shadow-lg";
const itemClass =
  "flex cursor-default items-center gap-2 rounded px-2 py-1 text-sm text-ink outline-none " +
  "focus:bg-surface-hover";

/** How long a finger rests on the link before its menu opens instead. */
const LONG_PRESS_MS = 450;

/**
 * A link to a deployment the user uses, shown as a chip naming what it leads to rather than
 * its address: the names of what each of its ids stands for, from the outside in, between
 * carets (a community, its channel, and whose message), led by the deployment's domain when
 * that is not the deployment the message is shown from. Pressing it opens that route here, as
 * any link within the app does, without loading the page again. Its address is its tooltip,
 * and right-clicking it, or holding a finger on it, offers the address itself: to copy, or to
 * open as an ordinary link would.
 */
export function SelfLink({ target, href }: { target: SelfLinkTarget; href: string }) {
  const m = useMessages();
  const shownFrom = useContext(ScopeDomainContext);
  const home = useContext(HomeClientContext);
  const source = useSources().find((s) => s.domain === target.domain);
  const anchor = useRef<HTMLAnchorElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const pressing = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pressedLong = useRef(false);
  const endPress = () => {
    clearTimeout(pressing.current);
  };
  const openMenu = () => {
    setMenuOpen(true);
  };
  const domain =
    target.domain ?? (home === null ? window.location.host : new URL(home.baseUrl).host);
  // The domain leads where the link is to another deployment than the message's, and stands
  // alone for the deployment's front page.
  const prefix = target.domain !== shownFrom || target.route.kind === "deployment" ? domain : null;
  return (
    <>
      <Link
        ref={anchor}
        {...selfLinkRoute(target)}
        title={href}
        aria-description={format(m.selfLinks.description, { url: href })}
        className={chipClass}
        onClick={(event) => {
          // A press held long enough to open the menu is not also a click.
          if (pressedLong.current) {
            pressedLong.current = false;
            event.preventDefault();
          }
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          openMenu();
        }}
        onPointerDown={(event) => {
          pressedLong.current = false;
          if (event.pointerType === "mouse") {
            return;
          }
          // The message's own long press, which offers its actions, is not this one.
          event.stopPropagation();
          pressing.current = setTimeout(() => {
            pressedLong.current = true;
            feelPress();
            openMenu();
          }, LONG_PRESS_MS);
        }}
        onPointerUp={endPress}
        onPointerCancel={endPress}
        onPointerLeave={endPress}
      >
        {source === undefined ? (
          <Path names={unnamed(m, target.route)} prefix={prefix} />
        ) : (
          <SourceScope source={source}>
            <RoutePath route={target.route} prefix={prefix} />
          </SourceScope>
        )}
      </Link>
      <Popover
        triggerRef={anchor}
        isOpen={menuOpen}
        onOpenChange={setMenuOpen}
        placement="bottom start"
        className={popoverClass}
      >
        <Dialog aria-label={m.selfLinks.options} className="outline-none">
          <Menu aria-label={m.selfLinks.options} className="outline-none">
            <MenuItem
              id="copy"
              className={itemClass}
              onAction={() => {
                void copyText(href, anchor.current ?? document.body).then((copied) => {
                  if (copied) {
                    toast(m.selfLinks.copiedLink);
                  }
                });
              }}
            >
              <CopyIcon size={14} aria-hidden="true" className="shrink-0 text-ink-muted" />
              {m.selfLinks.copyLink}
            </MenuItem>
            <MenuItem
              id="open"
              href={href}
              target="_blank"
              rel="noreferrer noopener"
              className={itemClass}
            >
              <ArrowSquareOutIcon
                size={14}
                aria-hidden="true"
                className="shrink-0 text-ink-muted"
              />
              {m.selfLinks.openOriginal}
            </MenuItem>
          </Menu>
        </Dialog>
      </Popover>
    </>
  );
}

/**
 * Names in order, `prefix` first when given, with a caret between each two, which a screen
 * reader hears as the comma that ends each name but the last.
 */
function Path({ names, prefix }: { names: readonly ReactNode[]; prefix: string | null }) {
  const all = prefix === null ? names : [prefix, ...names];
  return (
    <>
      {all.map((name, index) => (
        <Fragment key={index}>
          {index > 0 && (
            <CaretRightIcon
              size={12}
              weight="bold"
              aria-hidden="true"
              className="mx-1 inline align-[-0.05em] rtl:-scale-x-100"
            />
          )}
          <span className="break-words">
            {name}
            {index < all.length - 1 && <span className="sr-only">,</span>}
          </span>
        </Fragment>
      ))}
    </>
  );
}

/**
 * What a route is called by the records its ids name, read from the store of the deployment
 * in scope, each name a word for what it is while that is unknown or not the reader's to see.
 */
function RoutePath({ route, prefix }: { route: SelfRoute; prefix: string | null }) {
  const m = useMessages();
  const place = (community: string | null, channel: string): ReactNode[] =>
    community === null
      ? [<DmName key="dm" id={channel} />]
      : [
          <CommunityName key="community" id={community} />,
          <ChannelName key="channel" id={channel} />,
        ];
  switch (route.kind) {
    case "community":
      return (
        <Path names={[<CommunityName key="community" id={route.community} />]} prefix={prefix} />
      );
    case "channel":
      return <Path names={place(route.community, route.channel)} prefix={prefix} />;
    case "message":
      return (
        <Path
          names={[
            ...place(route.community, route.channel),
            <MessageName key="message" id={route.message} community={route.community} />,
          ]}
          prefix={prefix}
        />
      );
    case "thread":
      return (
        <Path
          names={[
            ...place(route.community, route.channel),
            // A thread has no name of its own; its starter message names it.
            m.selfLinks.thread,
          ]}
          prefix={prefix}
        />
      );
    case "botAdd":
      return (
        <Path
          names={[<PersonName key="bot" id={route.bot} />, m.selfLinks.addBot]}
          prefix={prefix}
        />
      );
    default:
      return <Path names={unnamed(m, route)} prefix={prefix} />;
  }
}

/** What a route is called without reading any record: a word for each thing it names. */
function unnamed(m: Messages, route: SelfRoute): ReactNode[] {
  const place = (community: string | null) =>
    community === null ? [m.selfLinks.directMessage] : [m.selfLinks.community, m.selfLinks.channel];
  switch (route.kind) {
    case "deployment":
      return [];
    case "community":
      return [m.selfLinks.community];
    case "channel":
      return place(route.community);
    case "message":
      return [...place(route.community), m.selfLinks.message];
    case "thread":
      return [...place(route.community), m.selfLinks.thread];
    case "dms":
      return [m.selfLinks.directMessages];
    case "invite":
      return [m.selfLinks.invite, route.code];
    case "registration":
      return [m.selfLinks.registrationInvite, route.code];
    case "deviceLink":
      return [m.selfLinks.signInCode];
    case "admin":
      return route.tab !== null && isAdminTab(route.tab)
        ? [m.admin.title, adminTabLabel(m, route.tab)]
        : [m.admin.title];
    case "botAdd":
      return [m.selfLinks.bot, m.selfLinks.addBot];
    case "attributions":
      return [m.about.attributions];
  }
}

function CommunityName({ id }: { id: string }) {
  const m = useMessages();
  return <>{useCommunity(id)?.name ?? m.selfLinks.community}</>;
}

/** A word-sized stand-in for a name on its way. */
function NameSkeleton() {
  return (
    <>
      <LoadingLabel />
      <Skeleton inline className="w-16" />
    </>
  );
}

/** A community channel's name; every channel the reader may see is in the store. */
function ChannelName({ id }: { id: string }) {
  const m = useMessages();
  return <>{useChannel(id)?.name ?? m.selfLinks.channel}</>;
}

/** A DM by the names of the people in it, read on demand when the store lacks it. */
function DmName({ id }: { id: string }) {
  const m = useMessages();
  const channel = useChannelOnDemand(id);
  const loading = useChannelLoading(id);
  if (channel !== undefined) {
    return <DmTitle channel={channel} />;
  }
  return loading ? <NameSkeleton /> : <>{m.selfLinks.directMessage}</>;
}

function DmTitle({ channel }: { channel: Channel }) {
  return <>{useDmTitle(channel)}</>;
}

/**
 * A message by whom it is from, as the store holds it: a message a body links to comes with
 * the read of that body (`include=linked`), which also says when it is deleted.
 */
function MessageName({ id, community }: { id: string; community: string | null }) {
  const m = useMessages();
  const message = useMessage(id);
  const linked = useLinkedMessage(id);
  if (message !== undefined) {
    return (
      <>
        {formatNodes(m.selfLinks.messageFrom, {
          name: <PersonName id={message.author} community={community} />,
        })}
      </>
    );
  }
  return <>{linked?.state === "deleted" ? m.selfLinks.deletedMessage : m.selfLinks.message}</>;
}
