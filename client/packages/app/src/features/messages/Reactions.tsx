import { ApiProblemError } from "@aspen/protocol";
import { SmileyIcon } from "@phosphor-icons/react";
import { lazy, Suspense, useState } from "react";
import { Button, Dialog, DialogTrigger, Popover, ToggleButton } from "react-aria-components";
import { useMe, useReactions, useStore, useSync } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The emoji picker is a sizeable chunk, fetched the first time anyone opens it. */
const EmojiPicker = lazy(() => import("@/features/messages/EmojiPicker"));

const chipClass =
  "flex items-center gap-1 rounded-full border px-2 py-0.5 text-sm outline-none " +
  "pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50";

/** The reaction chips under a message. Clicking one adds or removes the caller's own. */
export function ReactionChips({ messageId }: { messageId: string }) {
  const m = useMessages();
  const sync = useSync();
  const store = useStore();
  const me = useMe();
  const reactions = useReactions(messageId);
  if (reactions.size === 0) {
    return null;
  }
  return (
    <ul aria-label={m.reactionsLabel} className="mt-1 flex flex-wrap gap-1">
      {Array.from(reactions, ([emoji, users]) => {
        const mine = me !== null && users.has(me.id);
        const names = Array.from(users, (id) => {
          const user = store.user(id);
          return user === undefined ? m.unknownUser : displayNameOf(user);
        });
        return (
          <li key={emoji} title={format(m.reactedBy, { names: names.join(", "), emoji })}>
            <ToggleButton
              isSelected={mine}
              aria-label={format(mine ? m.youReactedWith : m.reactWith, { emoji })}
              onChange={(selected) => {
                void (
                  selected
                    ? sync.addReaction(messageId, emoji)
                    : sync.removeReaction(messageId, emoji)
                ).catch(() => undefined);
              }}
              className={
                chipClass +
                (mine
                  ? " border-accent bg-accent-soft text-accent-strong"
                  : " border-line bg-surface-raised hover:bg-surface-hover")
              }
            >
              <span>{emoji}</span>
              <span className="tabular-nums">{users.size}</span>
            </ToggleButton>
          </li>
        );
      })}
    </ul>
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
