import type { Channel } from "@aspen/protocol";
import { BellSlashIcon, NotePencilIcon, UsersThreeIcon } from "@phosphor-icons/react";
import { useId, useRef, useState } from "react";
import { Link, Outlet, useNavigate, useParams } from "@tanstack/react-router";
import { Button, Label, RadioButton, RadioField, RadioGroup } from "react-aria-components";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, useSources, type Source } from "@/api/everywhere";
import { useDms, useMe, useMentions, useMute, useUnread, useUser } from "@/api/hooks";
import { mergeDms } from "@/features/dms/mergeDms";
import { channelLink, useDomain } from "@/features/messages/links";
import { MentionBadge } from "@/features/mentions/MentionBadge";
import { mentionsText } from "@/features/mentions/mentions";
import { ChannelMenu, ChannelMenuButton } from "@/features/channels/ChannelMenu";
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

/**
 * `/dms`: the caller's DMs and group DMs beside the route's content, on their home and every
 * other deployment they use together, the most recently active first. On narrow screens only
 * one of the two is shown, as in a community.
 */
export function DmLayout() {
  const onePane = useOnePane();
  const { channelId } = useParams({ strict: false });
  const showing = channelId !== undefined;
  return (
    <>
      <div
        role={onePane && !showing ? "main" : undefined}
        className={`${showing ? "hidden md:flex" : "flex"} w-full flex-col md:w-64`}
      >
        <DmSidebar current={channelId} />
      </div>
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
      className="flex h-full flex-col border-e border-line bg-surface-raised"
    >
      <div className="flex items-center gap-2 border-b border-line px-4 py-2">
        <h1 id={headingId} className="min-w-0 flex-1 truncate font-semibold">
          {m.dms.label}
        </h1>
        <PeoplePicker
          trigger={
            <Tooltip text={m.dms.newMessage}>
              <Button
                aria-label={m.dms.newMessage}
                className="tap-target rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
              >
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
      </div>
      <nav
        aria-label={m.dms.label}
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-2"
      >
        {dms.map(({ dm, source }) => (
          <SourceScope key={`${source.domain ?? ""}/${dm.id}`} source={source}>
            <DmRow
              dm={dm}
              domain={source.domain}
              current={dm.id === current && source.domain === domain}
            />
          </SourceScope>
        ))}
      </nav>
      <SidebarFooter />
    </section>
  );
}

/**
 * One DM in the list, marked while it holds something the caller has not read, or dimmed with a
 * muted bell while they have muted it. Its menu opens on a right click or from its options
 * button, which sits over the row's right end, in room the link leaves for it. The row of the
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
  const muted = useMute(dm.id) !== undefined;
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
  const rowClass =
    "flex w-full items-center gap-2 rounded-md text-start text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
    (current
      ? "bg-surface-hover py-1.5 pe-8 ps-2 font-medium text-ink"
      : unread && !muted
        ? // The border and this padding make up the usual padding, so nothing moves.
          "py-[5px] pe-[31px] ps-[7px] " + unreadMarkClass
        : "py-1.5 pe-8 ps-2 " + (muted ? "text-ink-faint" : "text-ink-muted"));
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
      {muted && <BellSlashIcon size={14} aria-hidden="true" className="shrink-0" />}
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
      {/* Placed by a wrapper, since the button's own touch area keeps it `relative`. */}
      <span className="absolute top-1/2 end-2 flex -translate-y-1/2">
        <ChannelMenuButton
          name={title}
          isOpen={menuOpen}
          onPress={() => {
            setMenuOpen((open) => !open);
          }}
        />
      </span>
      <ChannelMenu
        channelId={dm.id}
        name={title}
        anchorRef={row}
        isOpen={menuOpen}
        onOpenChange={setMenuOpen}
      />
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
  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-1 p-6 text-center text-ink-muted">
      {dms.length === 0 && <p className="font-medium text-ink">{m.dms.empty}</p>}
      <p className="text-sm">{m.dms.emptyHint}</p>
    </main>
  );
}
