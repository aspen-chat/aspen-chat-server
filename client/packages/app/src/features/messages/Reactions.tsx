import { ACTION_ICON } from "@/features/messages/actionIcon";
import {
  ApiProblemError,
  REACTORS_PAGE,
  type EmojiReactions,
  type Reactions,
} from "@aspen/protocol";
import { useGrowthKey } from "@/features/layout/motion";
import { SmileyIcon, UsersIcon, XIcon } from "@phosphor-icons/react";
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Modal,
  ModalOverlay,
  Popover,
  type PopoverProps,
  Tab,
  TabList,
  TabPanel,
  Tabs,
  ToggleButton,
} from "react-aria-components";
import {
  useChannelCan,
  useCustomEmoji,
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
import { CustomEmojiGlyph } from "@/features/emoji/CustomEmojiGlyph";
import { emojiIdOf } from "@/features/emoji/customEmoji";
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

/** How long a touch on a reaction chip is held before it shows who reacted. */
const LONG_PRESS_MS = 500;

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
  communityId,
}: {
  messageId: string;
  /** Whether the caller may add reactions here; without it they may only take theirs back. */
  canReact: boolean;
  /** The community whose own emoji the reactions may be; none in a DM. */
  communityId: string | null;
}) {
  const m = useMessages();
  const reactions = useReactions(messageId);
  const [listOpen, setListOpen] = useState(false);
  /** The emoji whose people the list opens on, from a long press or right click on its chip. */
  const [listEmoji, setListEmoji] = useState<string | null>(null);
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
            communityId={communityId}
            emoji={emoji}
            reactions={r}
            canReact={canReact}
            fresh={!first.has(emoji)}
            onShowWho={() => {
              setListEmoji(emoji);
              setListOpen(true);
            }}
          />
        </li>
      ))}
      {hidden > 0 && (
        <li>
          <Tooltip text={format(m.moreReactions, { count: String(hidden) })}>
            <Button
              aria-label={format(m.moreReactions, { count: String(hidden) })}
              onPress={() => {
                setListEmoji(null);
                setListOpen(true);
              }}
              className={plainChipClass + " tabular-nums"}
            >
              +{hidden}
            </Button>
          </Tooltip>
        </li>
      )}
      {canReact && (
        <li>
          <ReactionPicker
            messageId={messageId}
            communityId={communityId}
            triggerClassName={plainChipClass + " text-ink-muted"}
          />
        </li>
      )}
      <ReactionsDialog
        messageId={messageId}
        communityId={communityId}
        isOpen={listOpen}
        onOpenChange={setListOpen}
        {...(listEmoji === null ? {} : { initialEmoji: listEmoji })}
      />
    </ul>
  );
}

/**
 * One emoji's chip. Its tooltip names the first `REACTION_SUMMARY_USERS` to react with it and
 * counts the rest. A chip `fresh` on a message already shown pops in, and its count pops each
 * time it grows. A right click, or a long press on a touch screen, shows everyone who reacted
 * with it (`onShowWho`) instead of adding or taking the reader's own.
 */
function ReactionChip({
  messageId,
  communityId,
  emoji,
  reactions,
  canReact,
  fresh,
  onShowWho,
}: {
  messageId: string;
  communityId: string | null;
  emoji: string;
  reactions: EmojiReactions;
  canReact: boolean;
  fresh: boolean;
  onShowWho: () => void;
}) {
  const pressing = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  /** Whether the press now ending was a long one, which showed who reacted and toggles nothing. */
  const pressedLong = useRef(false);
  const endPress = () => {
    clearTimeout(pressing.current);
  };
  const grown = useGrowthKey(reactions.count);
  const m = useMessages();
  const sync = useSync();
  const users = useUsers(reactions.users);
  const names = users.map((user) => (user === undefined ? m.unknownUser : displayNameOf(user)));
  const more = reactions.count - names.length;
  const named = useEmojiName(communityId, emoji);
  const who =
    more > 0
      ? format(m.reactedByMore, { names: names.join(", "), count: String(more), emoji: named })
      : format(m.reactedBy, { names: names.join(", "), emoji: named });
  return (
    <span
      className="block select-none [-webkit-touch-callout:none]"
      onContextMenu={(event) => {
        event.preventDefault();
        onShowWho();
      }}
      onPointerDownCapture={(event) => {
        pressedLong.current = false;
        if (event.pointerType !== "mouse") {
          pressing.current = setTimeout(() => {
            pressedLong.current = true;
            onShowWho();
          }, LONG_PRESS_MS);
        }
      }}
      onPointerUpCapture={endPress}
      onPointerCancelCapture={endPress}
      onPointerLeave={endPress}
    >
      <Tooltip text={who}>
        <ToggleButton
          isSelected={reactions.me}
          isDisabled={!reactions.me && !canReact}
          aria-label={format(reactions.me ? m.youReactedWith : m.reactWith, { emoji: named })}
          onChange={(selected) => {
            if (pressedLong.current) {
              pressedLong.current = false;
              return;
            }
            void (
              selected ? sync.addReaction(messageId, emoji) : sync.removeReaction(messageId, emoji)
            ).catch(() => undefined);
          }}
          className={
            (fresh ? "motion-pop " : "") +
            // The reader's own reaction is marked by a darker outline alone, in the chips' neutral
            // colours, so a message's reactions never outshine the message.
            (reactions.me
              ? chipClass + " border-ink-faint bg-surface-raised hover:bg-surface-hover"
              : plainChipClass)
          }
        >
          <EmojiKey emoji={emoji} communityId={communityId} />
          <span
            key={grown}
            className={"tabular-nums" + (grown > 0 ? " motion-pop inline-block" : "")}
          >
            {reactions.count}
          </span>
        </ToggleButton>
      </Tooltip>
    </span>
  );
}

/** The "View reactions" control in a message's toolbar, while the message has any. */
export function ViewReactionsButton({
  messageId,
  communityId,
  triggerClassName,
}: {
  messageId: string;
  communityId: string | null;
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
          <UsersIcon size={ACTION_ICON} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ReactionsDialog
        messageId={messageId}
        communityId={communityId}
        isOpen={open}
        onOpenChange={setOpen}
      />
    </>
  );
}

/**
 * Every reaction to a message: each emoji with its count, most popular first, and everyone who
 * reacted with the chosen one, earliest first, read a page at a time. The emoji run down the
 * side on a wide screen and across the top on a narrow one.
 */
export function ReactionsDialog({
  messageId,
  communityId,
  isOpen,
  onOpenChange,
  initialEmoji,
}: {
  messageId: string;
  /** The community whose own emoji the reactions may be; none in a DM. */
  communityId: string | null;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  /** The emoji whose people it opens on; the most popular without one. */
  initialEmoji?: string;
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
              {...(initialEmoji === undefined ? {} : { defaultSelectedKey: initialEmoji })}
              orientation={wide ? "vertical" : "horizontal"}
              className="flex min-h-0 flex-col gap-3 md:flex-row"
            >
              <TabList
                aria-label={m.reactionsLabel}
                items={entries}
                className="flex shrink-0 gap-1 overflow-x-auto md:max-h-96 md:w-28 md:flex-col md:overflow-x-visible md:overflow-y-auto"
              >
                {({ emoji, reactions: r }) => (
                  <ReactionTab emoji={emoji} communityId={communityId} count={r.count} />
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

/** One emoji's tab in the reactions dialog: the emoji and how many reacted with it. */
function ReactionTab({
  emoji,
  communityId,
  count,
}: {
  emoji: string;
  communityId: string | null;
  count: number;
}) {
  const m = useMessages();
  const named = useEmojiName(communityId, emoji);
  return (
    <Tab
      id={emoji}
      aria-label={format(m.reactionCount, { emoji: named, count: String(count) })}
      className="flex shrink-0 cursor-default items-center justify-between gap-2 rounded-md px-2 py-1 text-sm outline-none hover:bg-surface-hover selected:bg-accent-soft selected:text-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <span>
        <EmojiKey emoji={emoji} communityId={communityId} />
      </span>
      <span className="tabular-nums">{count}</span>
    </Tab>
  );
}

/** A reaction's emoji as drawn: the glyph, or a custom emoji's picture. */
function EmojiKey({ emoji, communityId }: { emoji: string; communityId: string | null }) {
  const id = emojiIdOf(emoji);
  return id === null ? (
    <span className="text-[1.5em] leading-none">{emoji}</span>
  ) : (
    <CustomEmojiGlyph id={id} communityId={communityId} size="large" />
  );
}

/** A reaction's emoji as spoken: the glyph, or a custom emoji's `:name:`. */
function useEmojiName(communityId: string | null, emoji: string): string {
  const m = useMessages();
  const custom = useCustomEmoji(communityId ?? "");
  const id = emojiIdOf(emoji);
  if (id === null) {
    return emoji;
  }
  const name = custom.find((e) => e.id === id)?.name;
  return name === undefined ? m.emoji.unknown : `:${name}:`;
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
  communityId,
  triggerClassName,
  iconSize = 16,
}: {
  messageId: string;
  /** The community whose own emoji the picker offers too; none in a DM. */
  communityId: string | null;
  triggerClassName: string;
  /** The trigger's icon size: a chip's beside the reactions, an action's in the actions. */
  iconSize?: number;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.addReaction}>
        <Button className={triggerClassName} aria-label={m.addReaction}>
          <SmileyIcon size={iconSize} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ReactionPickerPopover
        messageId={messageId}
        communityId={communityId}
        placement="bottom end"
      />
    </DialogTrigger>
  );
}

/**
 * The emoji picker that adds a reaction to the message, in a popover: opened by the trigger
 * around it, or, given `isOpen` and a `triggerRef`, by whatever holds it (a touch screen's
 * message actions, which close as it opens). A pick closes it.
 */
export function ReactionPickerPopover({
  messageId,
  communityId,
  ...popover
}: { messageId: string; communityId: string | null } & Omit<
  PopoverProps,
  "children" | "className"
>) {
  const m = useMessages();
  const sync = useSync();
  const [error, setError] = useState<string | null>(null);
  return (
    <>
      <Popover {...popover} className="rounded-lg border border-line bg-surface-raised shadow-lg">
        <Dialog aria-label={m.addReaction} className="outline-none">
          {({ close }) => (
            // A pick closes the popover: through the trigger's state around it, or, opened by
            // what holds it, through its own `onOpenChange`, which the dialog's `close` does
            // not reach.
            <div className="flex flex-col">
              <Suspense
                fallback={
                  <div className="flex h-96 w-80 items-center justify-center text-sm text-ink-muted">
                    {m.loading}
                  </div>
                }
              >
                <EmojiPicker
                  communityId={communityId}
                  onPick={(emoji) => {
                    setError(null);
                    sync.addReaction(messageId, emoji).then(
                      () => {
                        close();
                        popover.onOpenChange?.(false);
                      },
                      (e: unknown) => {
                        setError(e instanceof ApiProblemError ? e.message : String(e));
                      },
                    );
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
    </>
  );
}
