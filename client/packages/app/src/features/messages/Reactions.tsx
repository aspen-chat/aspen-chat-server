import {
  ApiProblemError,
  REACTORS_PAGE,
  type EmojiReactions,
  type Reactions,
} from "@aspen/protocol";
import { useGrowthKey } from "@/features/layout/motion";
import { SmileyIcon, UsersIcon, XIcon } from "@phosphor-icons/react";
import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Modal,
  ModalOverlay,
  Popover,
  Tab,
  TabList,
  TabPanel,
  Tabs,
  ToggleButton,
} from "react-aria-components";
import {
  useChannelCan,
  useMe,
  useMessage,
  useReactions,
  useSync,
  useUser,
  useUsers,
} from "@/api/hooks";
import {
  dialogClass,
  overlayClass,
  secondaryButtonClass,
  wideModalClass,
} from "@/features/invites/dialog";
import { MEDIUM_SCREEN, useMediaQuery } from "@/features/layout/useMediaQuery";
import { Tooltip } from "@/features/layout/Tooltip";
import { displayNameOf } from "@/features/users/profile";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { PersonAvatar, PersonName } from "@/features/users/PersonName";
import { RowsSkeleton } from "@/features/layout/ScreenSkeletons";

/** The emoji picker is a sizeable chunk, fetched the first time anyone opens it. */
const EmojiPicker = lazy(() => import("@/features/messages/EmojiPicker"));

const chipClass =
  "flex items-center gap-1 rounded-full border px-2 py-0.5 text-sm outline-none " +
  "pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50";
const plainChipClass = chipClass + " border-line bg-surface-raised hover:bg-surface-hover";

/** How many emoji show as chips under a message; the rest are counted in a "+N" chip. */
const VISIBLE_REACTIONS = 20;

interface Entry {
  emoji: string;
  reactions: EmojiReactions;
}

/** A message's reactions, most popular first; equally popular ones in the order first used. */
function byPopularity(reactions: Reactions): Entry[] {
  return Array.from(reactions, ([emoji, r]) => ({ emoji, reactions: r })).sort(
    (a, b) => b.reactions.count - a.reactions.count,
  );
}

/**
 * The reaction chips under a message, most popular first: at most `VISIBLE_REACTIONS`, then a
 * chip counting the rest that opens the full list, then a chip that adds one. Clicking an emoji
 * chip adds or removes the caller's own.
 */
export function ReactionChips({
  messageId,
  canReact,
}: {
  messageId: string;
  /** Whether the caller may add reactions here; without it they may only take theirs back. */
  canReact: boolean;
}) {
  const m = useMessages();
  const reactions = useReactions(messageId);
  const [listOpen, setListOpen] = useState(false);
  // The emoji the message had when drawn; one added while it is on screen pops in.
  const [first] = useState(() => new Set(reactions.keys()));
  if (reactions.size === 0) {
    return null;
  }
  const entries = byPopularity(reactions);
  const shown = entries.slice(0, VISIBLE_REACTIONS);
  const hidden = entries.length - shown.length;
  return (
    <ul aria-label={m.reactionsLabel} className="mt-1 flex flex-wrap gap-1">
      {shown.map(({ emoji, reactions: r }) => (
        <li key={emoji}>
          <ReactionChip
            messageId={messageId}
            emoji={emoji}
            reactions={r}
            canReact={canReact}
            fresh={!first.has(emoji)}
          />
        </li>
      ))}
      {hidden > 0 && (
        <li>
          <Tooltip text={format(m.moreReactions, { count: String(hidden) })}>
            <Button
              aria-label={format(m.moreReactions, { count: String(hidden) })}
              onPress={() => {
                setListOpen(true);
              }}
              className={plainChipClass + " tabular-nums"}
            >
              +{hidden}
            </Button>
          </Tooltip>
          <ReactionsDialog messageId={messageId} isOpen={listOpen} onOpenChange={setListOpen} />
        </li>
      )}
      {canReact && (
        <li>
          <ReactionPicker
            messageId={messageId}
            triggerClassName={plainChipClass + " text-ink-muted"}
          />
        </li>
      )}
    </ul>
  );
}

/**
 * One emoji's chip. Its tooltip names the first `REACTION_SUMMARY_USERS` to react with it and
 * counts the rest. A chip `fresh` on a message already shown pops in, and its count pops each
 * time it grows.
 */
function ReactionChip({
  messageId,
  emoji,
  reactions,
  canReact,
  fresh,
}: {
  messageId: string;
  emoji: string;
  reactions: EmojiReactions;
  canReact: boolean;
  fresh: boolean;
}) {
  const grown = useGrowthKey(reactions.count);
  const m = useMessages();
  const sync = useSync();
  const users = useUsers(reactions.users);
  const names = users.map((user) => (user === undefined ? m.unknownUser : displayNameOf(user)));
  const more = reactions.count - names.length;
  const who =
    more > 0
      ? format(m.reactedByMore, { names: names.join(", "), count: String(more), emoji })
      : format(m.reactedBy, { names: names.join(", "), emoji });
  return (
    <Tooltip text={who}>
      <ToggleButton
        isSelected={reactions.me}
        isDisabled={!reactions.me && !canReact}
        aria-label={format(reactions.me ? m.youReactedWith : m.reactWith, { emoji })}
        onChange={(selected) => {
          void (
            selected ? sync.addReaction(messageId, emoji) : sync.removeReaction(messageId, emoji)
          ).catch(() => undefined);
        }}
        className={
          (fresh ? "motion-pop " : "") +
          (reactions.me
            ? chipClass + " border-accent bg-accent-soft text-accent-strong"
            : plainChipClass)
        }
      >
        <span>{emoji}</span>
        <span
          key={grown}
          className={"tabular-nums" + (grown > 0 ? " motion-pop inline-block" : "")}
        >
          {reactions.count}
        </span>
      </ToggleButton>
    </Tooltip>
  );
}

/** The "View reactions" control in a message's toolbar, while the message has any. */
export function ViewReactionsButton({
  messageId,
  triggerClassName,
}: {
  messageId: string;
  triggerClassName: string;
}) {
  const m = useMessages();
  const reactions = useReactions(messageId);
  const [open, setOpen] = useState(false);
  if (reactions.size === 0) {
    return null;
  }
  return (
    <>
      <Tooltip text={m.viewReactions}>
        <Button
          aria-label={m.viewReactions}
          onPress={() => {
            setOpen(true);
          }}
          className={triggerClassName}
        >
          <UsersIcon size={16} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ReactionsDialog messageId={messageId} isOpen={open} onOpenChange={setOpen} />
    </>
  );
}

/**
 * Every reaction to a message: each emoji with its count, most popular first, and everyone who
 * reacted with the chosen one, earliest first, read a page at a time. The emoji run down the
 * side on a wide screen and across the top on a narrow one.
 */
function ReactionsDialog({
  messageId,
  isOpen,
  onOpenChange,
}: {
  messageId: string;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const reactions = useReactions(messageId);
  const wide = useMediaQuery(MEDIUM_SCREEN);
  const entries = byPopularity(reactions);
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={wideModalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{m.reactionsHeading}</DialogHeading>
          {entries.length === 0 ? (
            <p className="text-sm text-ink-muted">{m.noReactions}</p>
          ) : (
            <Tabs
              orientation={wide ? "vertical" : "horizontal"}
              className="flex min-h-0 flex-col gap-3 md:flex-row"
            >
              <TabList
                aria-label={m.reactionsLabel}
                items={entries}
                className="flex shrink-0 gap-1 overflow-x-auto md:max-h-96 md:w-28 md:flex-col md:overflow-x-visible md:overflow-y-auto"
              >
                {({ emoji, reactions: r }) => (
                  <Tab
                    id={emoji}
                    aria-label={format(m.reactionCount, { emoji, count: String(r.count) })}
                    className="flex shrink-0 cursor-default items-center justify-between gap-2 rounded-md px-2 py-1 text-sm outline-none hover:bg-surface-hover selected:bg-accent-soft selected:text-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50"
                  >
                    <span>{emoji}</span>
                    <span className="tabular-nums">{r.count}</span>
                  </Tab>
                )}
              </TabList>
              {entries.map(({ emoji }) => (
                <TabPanel key={emoji} id={emoji} className="min-w-0 flex-1 outline-none">
                  <ReactorList messageId={messageId} emoji={emoji} />
                </TabPanel>
              ))}
            </Tabs>
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** Everyone who reacted with one emoji, earliest first, read a page at a time. */
function ReactorList({ messageId, emoji }: { messageId: string; emoji: string }) {
  const m = useMessages();
  const sync = useSync();
  // The ids of a paged read, in the server's order; the records themselves are in the store.
  const [ids, setIds] = useState<readonly string[]>([]);
  const [complete, setComplete] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(
    (after: string | undefined, isCurrent: () => boolean) => {
      sync.loadReactors(messageId, emoji, after).then(
        (users) => {
          if (!isCurrent()) {
            return;
          }
          const page = users.map((u) => u.id);
          setIds((held) => (after === undefined ? page : [...held, ...page]));
          setComplete(users.length < REACTORS_PAGE);
          setLoading(false);
        },
        (e: unknown) => {
          if (isCurrent()) {
            setError(e instanceof ApiProblemError ? e.message : String(e));
            setLoading(false);
          }
        },
      );
    },
    [sync, messageId, emoji],
  );

  useEffect(() => {
    let current = true;
    read(undefined, () => current);
    return () => {
      current = false;
    };
  }, [read]);

  function showMore() {
    setLoading(true);
    setError(null);
    read(ids[ids.length - 1], () => true);
  }

  return (
    <div className="flex max-h-96 flex-col gap-1 overflow-y-auto">
      <ul aria-label={format(m.reactedWithLabel, { emoji })} className="flex flex-col gap-1">
        {ids.map((id) => (
          <Reactor
            key={id}
            userId={id}
            messageId={messageId}
            emoji={emoji}
            onRemoved={() => {
              setIds((held) => held.filter((other) => other !== id));
            }}
          />
        ))}
      </ul>
      {loading && <RowsSkeleton count={ids.length === 0 ? 5 : 2} />}
      {error !== null && (
        <p role="alert" className="px-1 text-sm text-danger">
          {error}
        </p>
      )}
      {!loading && !complete && ids.length > 0 && (
        <Button onPress={showMore} className={secondaryButtonClass + " self-start"}>
          {m.showMore}
        </Button>
      )}
    </div>
  );
}

/**
 * One person who reacted, with, for those who may manage messages where it is, a control that
 * takes their reaction off.
 */
function Reactor({
  userId,
  messageId,
  emoji,
  onRemoved,
}: {
  userId: string;
  messageId: string;
  emoji: string;
  onRemoved: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const user = useUser(userId);
  const me = useMe();
  const channelId = useMessage(messageId)?.channelId ?? "";
  const moderate = useChannelCan(channelId, "manageMessages") && me?.id !== userId;
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  const label = format(m.removeReactor, { name });
  return (
    <li className="flex items-center gap-2 rounded-md px-1 py-1 text-sm">
      <PersonAvatar id={userId} size="sm" />
      <span className="min-w-0 flex-1 truncate">
        <PersonName id={userId} />
      </span>
      {moderate && (
        <Tooltip text={label}>
          <Button
            aria-label={label}
            onPress={() => {
              sync.removeUsersReaction(messageId, emoji, userId).then(onRemoved, () => undefined);
            }}
            className="tap-target rounded p-1 text-ink-muted outline-none hover:text-danger focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <XIcon size={14} aria-hidden="true" />
          </Button>
        </Tooltip>
      )}
    </li>
  );
}

/** The "React" control in a message's hover toolbar and the picker it opens. */
export function ReactionPicker({
  messageId,
  triggerClassName,
}: {
  messageId: string;
  triggerClassName: string;
}) {
  const m = useMessages();
  const sync = useSync();
  const [error, setError] = useState<string | null>(null);
  return (
    <DialogTrigger>
      <Tooltip text={m.addReaction}>
        <Button className={triggerClassName} aria-label={m.addReaction}>
          <SmileyIcon size={16} aria-hidden="true" />
        </Button>
      </Tooltip>
      <Popover
        placement="bottom end"
        className="rounded-lg border border-line bg-surface-raised shadow-lg"
      >
        <Dialog aria-label={m.addReaction} className="outline-none">
          {({ close }) => (
            <div className="flex flex-col">
              <Suspense
                fallback={
                  <div className="flex h-96 w-80 items-center justify-center text-sm text-ink-muted">
                    {m.loading}
                  </div>
                }
              >
                <EmojiPicker
                  onPick={(emoji) => {
                    setError(null);
                    sync.addReaction(messageId, emoji).then(close, (e: unknown) => {
                      setError(e instanceof ApiProblemError ? e.message : String(e));
                    });
                  }}
                />
              </Suspense>
              {error !== null && (
                <p role="alert" className="px-3 pb-2 text-xs text-danger">
                  {error}
                </p>
              )}
            </div>
          )}
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}
