import { ApiProblemError } from "@aspen/protocol";
import { SmileyIcon } from "@phosphor-icons/react";
import { useMemo, type ReactNode } from "react";
import { ToggleButton } from "react-aria-components";
import { useCustomEmoji, useFrequentEmoji, useReactions, useSync } from "@/api/hooks";
import { Drawer } from "@/features/layout/Drawer";
import { IconAction } from "@/features/layout/IconAction";
import { Skeleton } from "@/features/layout/Skeleton";
import { toast } from "@/features/layout/toast";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { QUICK_REACTIONS, quickReactions } from "@/features/messages/quickReactions";
import { useEmojiName } from "@/features/messages/emojiName";
import { EmojiKey } from "@/features/messages/Reactions";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A quick reaction's button, and the Add a reaction button beside them: round, finger sized in
 * a touch screen's sheet, smaller in a context menu (`compact`).
 */
const quickClass =
  "flex shrink-0 items-center justify-center rounded-full border border-transparent " +
  "bg-surface-hover text-ink-muted outline-none pressed:opacity-80 " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";
const fingerSize = "h-12 w-12";
const compactSize = "h-8 w-8 text-xs [&_svg]:h-4.5 [&_svg]:w-4.5";

/**
 * A touch screen's actions on a message, opened by a long press on it (`MessageItem`): a sheet
 * sliding up from the bottom over a darkened screen, holding the quick reactions, where the
 * reader may react (`canReact`), when the message was sent (`sentAt`, which a message grouped
 * under another shows nowhere else on a touch screen), and below them `children`, the actions as a list of icons and
 * names (`MessageActions`). A quick reaction toggles the reader's reaction and closes the
 * sheet; the Add a reaction button beside them calls `onMore`, which opens the full picker in
 * the sheet's place.
 */
export function MessageActionSheet({
  messageId,
  communityId,
  sentAt,
  canReact,
  isOpen,
  onOpenChange,
  onMore,
  children,
}: {
  messageId: string;
  /** The community the message is in, whose own emoji may be among the quick reactions. */
  communityId: string | null;
  /** When the message was sent, as the reader reads dates and times. */
  sentAt: string;
  canReact: boolean;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  onMore: () => void;
  children: ReactNode;
}) {
  const m = useMessages();
  return (
    <Drawer edge="bottom" isOpen={isOpen} onOpenChange={onOpenChange} title={m.messageActionsLabel}>
      <div className="min-h-0 overflow-y-auto overscroll-contain pb-2">
        {canReact && (
          <QuickReactions
            messageId={messageId}
            communityId={communityId}
            onDone={() => {
              onOpenChange(false);
            }}
            onMore={onMore}
          />
        )}
        <p className="border-b border-line px-4 py-2 text-xs text-ink-muted">
          {format(m.sentAt, { time: sentAt })}
        </p>
        <div role="group" aria-label={m.messageActionsLabel} className="flex flex-col pt-1">
          {children}
        </div>
      </div>
    </Drawer>
  );
}

/**
 * The emoji the reader reacts with most here, topped up with the defaults (`quickReactions`),
 * each a toggle of their reaction, then the Add a reaction button. Until the reader's most used
 * are read, the room they will take. `compact`, they are drawn for a pointer, as in a message's
 * context menu, rather than for a finger.
 */
export function QuickReactions({
  messageId,
  communityId,
  compact = false,
  onDone,
  onMore,
}: {
  messageId: string;
  communityId: string | null;
  compact?: boolean;
  onDone: () => void;
  onMore: () => void;
}) {
  const size = compact ? compactSize : fingerSize;
  const m = useMessages();
  const frequent = useFrequentEmoji(communityId);
  const custom = useCustomEmoji(communityId ?? "");
  const usable = useMemo(() => new Set(custom.map((e) => e.id)), [custom]);
  const chosen = useMemo(
    () => (frequent === undefined ? undefined : quickReactions(frequent, usable)),
    [frequent, usable],
  );
  return (
    <div
      role="group"
      aria-label={m.quickReactions}
      aria-busy={chosen === undefined}
      className={
        "flex items-center justify-between " +
        (compact ? "gap-0.5 px-1 pt-0.5 pb-1.5" : "gap-2 px-4 pt-1 pb-2")
      }
    >
      {chosen === undefined
        ? Array.from({ length: QUICK_REACTIONS }, (_, i) => (
            <Skeleton key={i} className={size + " shrink-0 rounded-full"} />
          ))
        : chosen.map((emoji) => (
            <QuickReaction
              key={emoji}
              messageId={messageId}
              communityId={communityId}
              emoji={emoji}
              className={quickClass + " " + size}
              onDone={onDone}
            />
          ))}
      <IconAction
        label={m.addReaction}
        onPress={onMore}
        className={quickClass + " " + size}
        icon={<SmileyIcon size={ACTION_ICON} aria-hidden="true" />}
      />
    </div>
  );
}

/**
 * One quick reaction: pressed where the reader already reacted with it, so pressing it again
 * takes that reaction back, as its chip under the message would.
 */
function QuickReaction({
  messageId,
  communityId,
  emoji,
  className,
  onDone,
}: {
  messageId: string;
  communityId: string | null;
  emoji: string;
  className: string;
  onDone: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const mine = useReactions(messageId).get(emoji)?.me ?? false;
  const named = useEmojiName(communityId, emoji);
  return (
    <ToggleButton
      isSelected={mine}
      aria-label={format(mine ? m.youReactedWith : m.reactWith, { emoji: named })}
      onChange={(selected) => {
        onDone();
        (selected
          ? sync.addReaction(messageId, emoji)
          : sync.removeReaction(messageId, emoji)
        ).catch((e: unknown) => {
          toast(e instanceof ApiProblemError ? e.message : String(e));
        });
      }}
      className={className + " selected:border-ink-faint"}
    >
      <EmojiKey emoji={emoji} communityId={communityId} />
    </ToggleButton>
  );
}
