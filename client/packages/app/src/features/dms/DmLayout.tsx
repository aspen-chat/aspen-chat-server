import type { Channel } from "@aspen/protocol";
import { SidebarHeader } from "@/features/layout/SidebarHeader";
import { headerIconButtonClass } from "@/features/layout/headerButton";
import { PaneEdge, ResizablePane } from "@/features/layout/ResizablePane";
import { CHANNEL_LIST } from "@/features/layout/paneSizes";
import { NotePencilIcon, PhoneIcon, UsersThreeIcon } from "@phosphor-icons/react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Link, Outlet, useNavigate, useParams } from "@tanstack/react-router";
import { Button, Label, RadioButton, RadioField, RadioGroup } from "react-aria-components";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, useSources, type Source } from "@/api/everywhere";
import {
  useChannelVoice,
  useDms,
  useMe,
  useMentions,
  useMute,
  useSyncStatus,
  useUnread,
  useUser,
} from "@/api/hooks";
import { mergeDms } from "@/features/dms/mergeDms";
import { channelLink, useDomain } from "@/features/messages/links";
import { MentionBadge } from "@/features/mentions/MentionBadge";
import { mentionsText } from "@/features/mentions/mentions";
import { ChannelMenu, ChannelMenuButton } from "@/features/channels/ChannelMenu";
import { MuteBell } from "@/features/channels/MuteBell";
import { Avatar } from "@/features/communities/Avatar";
import { MAX_DM_PEOPLE } from "@/features/dms/DmHeader";
import { PeoplePicker } from "@/features/dms/PeoplePicker";
import { otherRecipients } from "@/features/dms/dmName";
import { useDmTitle } from "@/features/dms/useDmTitle";
import { SidebarFooter } from "@/features/layout/SidebarFooter";
import { Tooltip } from "@/features/layout/Tooltip";
import { unreadMarkClass } from "@/features/channels/ChannelSidebar";
import { RadioMark, choiceClass } from "@/features/layout/choices";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { RowsSkeleton } from "@/features/layout/ScreenSkeletons";
import { OnceOpen } from "@/features/layout/OnceOpen";

/**
 * `/dms`: the caller's DMs and group DMs beside the route's content, on their home and every
 * other deployment they use together, the most recently active first. On narrow screens only
 * one of the two is shown, as in a community.
 */
export function DmLayout() {
  const m = useMessages();
  const onePane = useOnePane();
  const { channelId } = useParams({ strict: false });
  const showing = channelId !== undefined;
  return (
    <>
      <ResizablePane
        sizing={CHANNEL_LIST}
        edge="end"
        label={m.layout.channelList}
        role={onePane && !showing ? "main" : undefined}
        className={`${showing ? "hidden md:flex" : "flex"} w-full flex-col`}
      >
        <DmSidebar current={channelId} />
      </ResizablePane>
      <div className={`${showing ? "flex" : "hidden md:flex"} min-w-0 flex-1 flex-col`}>
        <Outlet />
      </div>
    </>
  );
}

/** A DM in the list, with the deployment it is on. */
interface DmEntry {
  readonly dm: Channel;
  readonly source: Source;
}

function DmSidebar({ current }: { current: string | undefined }) {
  const m = useMessages();
  const headingId = useId();
  const domain = useDomain();
  const navigate = useNavigate();
  const sources = useSources();
  // A new conversation starts on the deployment shown unless the user picks another.
  const [chosen, setChosen] = useState<string | null>(domain);
  const host =
    sources.find((s) => s.domain === chosen) ??
    sources.find((s) => s.domain === domain) ??
    sources[0];
  const bootstrapping = useSyncStatus() === "bootstrapping";
  const dms = useEverywhere(["dms"], (sources) =>
    mergeDms<DmEntry>(
      sources.map((source) => ({
        dms: source.sync.store.dms().map((dm) => ({ dm, source })),
        activity: (entry) => source.sync.store.dmActivity(entry.dm.id),
      })),
    ),
  );
  return (
    // A landmark named by its heading, holding the DMs and the user's own controls.
    <section
      aria-labelledby={headingId}
      className="relative flex h-full flex-col border-e border-line bg-surface-raised"
    >
      <SidebarHeader headingId={headingId} title={m.dms.label}>
        <PeoplePicker
          trigger={
            <Tooltip text={m.dms.newMessage}>
              <Button aria-label={m.dms.newMessage} className={headerIconButtonClass}>
                <NotePencilIcon size={18} aria-hidden="true" />
              </Button>
            </Tooltip>
          }
          heading={m.dms.newMessage}
          confirmLabel={m.dms.start}
          pendingLabel={m.dms.starting}
          exclude={[]}
          max={MAX_DM_PEOPLE - 1}
          {...(host === undefined ? {} : { source: host })}
          above={
            sources.length > 1 && host !== undefined ? (
              <StartOn sources={sources} value={host.domain} onChange={setChosen} />
            ) : undefined
          }
          onConfirm={async (ids) => {
            if (host === undefined) {
              return;
            }
            const dm = await host.sync.openDm(ids);
            void navigate(channelLink({ domain: host.domain, community: null }, dm.id));
          }}
        />
      </SidebarHeader>
      <nav
        aria-label={m.dms.label}
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-2"
      >
        {dms.length === 0 && bootstrapping && <RowsSkeleton count={6} />}
        {dms.map(({ dm, source }) => (
          <SourceScope key={`${source.domain ?? ""}/${dm.id}`} source={source}>
            <DmRow
              dm={dm}
              domain={source.domain}
              current={dm.id === current && source.domain === domain}
            />
          </SourceScope>
        ))}
        <OlderDms />
      </nav>
      <SidebarFooter />
      <PaneEdge />
    </section>
  );
}

/**
 * The end of the DM list while any deployment has older DMs than it has listed: the next page
 * of each is read as this comes into view, or when it is pressed.
 */
function OlderDms() {
  const m = useMessages();
  const sources = useEverywhere(["dms"], (all) => all.filter((s) => !s.sync.store.dmsComplete()));
  const [loading, setLoading] = useState(false);
  const end = useRef<HTMLButtonElement>(null);
  const load = useCallback(() => {
    if (loading || sources.length === 0) {
      return;
    }
    setLoading(true);
    void Promise.all(
      sources.map((source) => source.sync.loadMoreDms().catch(() => undefined)),
    ).finally(() => {
      setLoading(false);
    });
  }, [loading, sources]);
  // Watched afresh after each page, so the next is read while the end stays in view.
  useEffect(() => {
    const element = end.current;
    if (element === null) {
      return;
    }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        load();
      }
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [load]);
  if (sources.length === 0) {
    return null;
  }
  return (
    <Button
      ref={end}
      onPress={load}
      isDisabled={loading}
      className="rounded-md px-2 py-1.5 text-start text-sm text-ink-muted outline-none hover:bg-surface-hover hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      {loading ? m.dms.loadingOlder : m.dms.older}
    </Button>
  );
}

/**
 * One DM in the list, marked while it holds something the caller has not read, or dimmed with a
 * muted bell, whose tooltip says until when, while they have muted it, and with a phone while a
 * call is under way in it. Its menu opens on a right click or from its options button, which
 * sits over the row's right end with the bell, in room the link leaves for them. The row of the
 * one-to-one DM already open opens the other person's card instead, beside the row, so an
 * unwanted conversation is a press away from a block.
 */
function DmRow({ dm, domain, current }: { dm: Channel; domain: string | null; current: boolean }) {
  const m = useMessages();
  const me = useMe();
  const title = useDmTitle(dm);
  const first = useUser(otherRecipients(dm, me?.id ?? null)[0]);
  const unread = useUnread(dm.id);
  const tags = useMentions(dm.id);
  const mute = useMute(dm.id);
  const muted = mute !== undefined;
  const calling = useChannelVoice(dm.id).participants.length > 0;
  const row = useRef<HTMLDivElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const stateName = muted
    ? format(m.mutedLabel, { name: title })
    : unread
      ? format(m.unreadLabel, { name: title })
      : null;
  const accessibleName =
    tags > 0
      ? format(m.withMentions, { name: stateName ?? title, mentions: mentionsText(m, tags) })
      : stateName;
  // The room the link leaves at its end for the options button, and for the bell beside it and
  // the gap before it while muted.
  const endRoom = muted ? "pe-[54px] " : "pe-8 ";
  const rowClass =
    "flex w-full items-center gap-2 rounded-md text-start text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
    (current
      ? "bg-surface-hover py-1.5 ps-2 font-medium text-ink " + endRoom
      : unread && !muted
        ? // The border and this padding make up the usual padding, so nothing moves.
          "py-[5px] pe-[31px] ps-[7px] " + unreadMarkClass
        : "py-1.5 ps-2 " + endRoom + (muted ? "text-ink-faint" : "text-ink-muted"));
  const content = (
    <>
      {dm.ty === "groupDm" ? (
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-surface-sunken text-ink-muted">
          <UsersThreeIcon size={16} aria-hidden="true" />
        </span>
      ) : (
        <Avatar name={first === undefined ? title : displayNameOf(first)} iconId={first?.icon} />
      )}
      {accessibleName === null ? (
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate">{title}</span>
          {domain !== null && <span className="truncate text-xs text-ink-faint">{domain}</span>}
        </span>
      ) : (
        <>
          <span aria-hidden="true" className="flex min-w-0 flex-1 flex-col">
            <span className="truncate">{title}</span>
            {domain !== null && <span className="truncate text-xs text-ink-faint">{domain}</span>}
          </span>
          <span className="sr-only">
            {domain === null
              ? accessibleName
              : format(m.deployments.onDomain, { name: accessibleName, domain })}
          </span>
        </>
      )}
      <MentionBadge count={tags} />
      {calling && (
        <PhoneIcon
          size={14}
          weight="fill"
          role="img"
          aria-label={m.voice.callUnderWay}
          className="shrink-0 text-online"
        />
      )}
    </>
  );
  return (
    <div
      ref={row}
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuOpen(true);
      }}
      className="group relative"
    >
      {current && dm.ty === "dm" && first !== undefined ? (
        <ProfilePopover user={first} placement="end" anchorRef={row}>
          <Button
            aria-current="page"
            aria-label={format(m.profile.show, { name: accessibleName ?? title })}
            className={rowClass}
          >
            {content}
          </Button>
        </ProfilePopover>
      ) : (
        <Link
          {...channelLink({ domain, community: null }, dm.id)}
          aria-current={current ? "page" : undefined}
          className={rowClass}
        >
          {content}
        </Link>
      )}
      {/* Placed by a wrapper, since the button's own touch area keeps it `relative`. The bell is
          here rather than in the link, which may not hold anything focusable. */}
      <span
        className={
          "absolute top-1/2 end-2 flex -translate-y-1/2 items-center gap-1.5" +
          (current ? " text-ink" : " text-ink-faint")
        }
      >
        {mute !== undefined && <MuteBell mute={mute} />}
        <ChannelMenuButton
          name={title}
          isOpen={menuOpen}
          onPress={() => {
            setMenuOpen((open) => !open);
          }}
        />
      </span>
      <OnceOpen isOpen={menuOpen}>
        <ChannelMenu
          channelId={dm.id}
          name={title}
          anchorRef={row}
          isOpen={menuOpen}
          onOpenChange={setMenuOpen}
        />
      </OnceOpen>
    </div>
  );
}

/**
 * Which deployment a new conversation starts on, among those the user is on: it hosts the
 * conversation, and offers the people the user shares a community with there.
 */
function StartOn({
  sources,
  value,
  onChange,
}: {
  sources: readonly Source[];
  value: string | null;
  onChange: (domain: string | null) => void;
}) {
  const m = useMessages();
  return (
    <RadioGroup
      value={value ?? HOME}
      onChange={(next) => {
        onChange(next === HOME ? null : next);
      }}
      className="flex flex-col gap-1.5"
    >
      <Label className="text-sm font-medium">{m.dms.startOn}</Label>
      <div className="flex flex-wrap gap-2">
        {sources.map((source) => (
          <RadioField key={source.domain ?? HOME} value={source.domain ?? HOME}>
            <RadioButton className={choiceClass}>
              <RadioMark />
              <span>{source.domain ?? m.dms.home}</span>
            </RadioButton>
          </RadioField>
        ))}
      </div>
    </RadioGroup>
  );
}

/** The home deployment among the radio values, which are otherwise domains. */
const HOME = "";

/** `/dms` with nothing chosen: how to start, when there is nothing yet. */
export function DmIndex() {
  const m = useMessages();
  const dms = useDms();
  const bootstrapping = useSyncStatus() === "bootstrapping";
  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-1 p-6 text-center text-ink-muted">
      {dms.length === 0 && !bootstrapping && <p className="font-medium text-ink">{m.dms.empty}</p>}
      <p className="text-sm">{m.dms.emptyHint}</p>
    </main>
  );
}
