import type { Poll } from "@aspen/protocol";
import { CheckIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { ToggleButton } from "react-aria-components";
import { useMyVotes, usePoll, useStore, useSync } from "@/api/hooks";
import { optionName, pollOpen, remainingText, sharePercent } from "@/features/messages/poll";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A poll in its message: the question, one bar per option showing its share of the votes, the
 * caller's own choices, and how long voting stays open. Voting is a toggle on each option
 * while the poll is open; the tally itself updates from the server's events.
 */
export function PollCard({ pollId }: { pollId: string }) {
  const m = useMessages();
  const poll = usePoll(pollId);
  if (poll === undefined) {
    return <p className="mt-1 text-sm text-ink-faint">{m.poll.unavailable}</p>;
  }
  return <LoadedPollCard poll={poll} />;
}

function LoadedPollCard({ poll }: { poll: Poll }) {
  const m = useMessages();
  const sync = useSync();
  const store = useStore();
  const myVotes = useMyVotes(poll.id);
  const now = useNow(poll);
  const open = pollOpen(poll, now);
  const total = poll.results.reduce((sum, r) => sum + r.count, 0);

  function toggle(option: number, selected: boolean) {
    void (selected ? sync.vote(poll.id, option) : sync.unvote(poll.id, option)).catch(
      () => undefined,
    );
  }

  return (
    <section
      aria-label={format(m.poll.label, { question: poll.question })}
      className="mt-1 flex w-full max-w-lg flex-col gap-2 rounded-md border border-line bg-surface-raised p-3"
    >
      <h3 className="font-medium">{poll.question}</h3>
      <ul className="flex flex-col gap-1.5">
        {poll.options.map((option, index) => {
          const result = poll.results[index];
          const mine = myVotes.has(index);
          const voters = result?.voters ?? null;
          const names =
            voters === null
              ? null
              : voters
                  .map((id) => {
                    const user = store.user(id);
                    return user === undefined ? m.unknownUser : displayNameOf(user);
                  })
                  .join(", ");
          return (
            <li key={index}>
              <ToggleButton
                isSelected={mine}
                isDisabled={!open}
                onChange={(selected) => {
                  toggle(index, selected);
                }}
                aria-label={format(mine ? m.poll.unvote : m.poll.vote, {
                  option: optionName(option),
                })}
                className={
                  "relative flex w-full items-center gap-2 overflow-hidden rounded-md border px-3 py-1.5 text-left text-sm outline-none " +
                  "focus-visible:ring-2 focus-visible:ring-accent/50 " +
                  (mine ? "border-accent" : "border-line") +
                  (open ? " hover:bg-surface-hover pressed:opacity-80" : " cursor-default")
                }
              >
                <span
                  aria-hidden="true"
                  className={
                    "absolute inset-y-0 left-0 transition-[width] " +
                    (mine ? "bg-accent-soft" : "bg-surface-sunken")
                  }
                  style={{ width: `${String(sharePercent(poll, index))}%` }}
                />
                <span className="relative flex min-w-0 flex-1 flex-col">
                  <span className="flex items-center gap-1.5">
                    {mine && <CheckIcon size={14} weight="bold" aria-hidden="true" />}
                    {option.emoji != null && (
                      <span className="text-base leading-none" aria-hidden="true">
                        {option.emoji}
                      </span>
                    )}
                    <span className="truncate">{option.label}</span>
                  </span>
                  {names !== null && names.length > 0 && (
                    <span className="truncate text-xs text-ink-muted">
                      {format(m.poll.votedBy, { names })}
                    </span>
                  )}
                </span>
                <span className="relative tabular-nums text-ink-muted">
                  {String(sharePercent(poll, index))}%
                </span>
              </ToggleButton>
            </li>
          );
        })}
      </ul>
      <p className="flex flex-wrap gap-x-3 text-xs text-ink-faint">
        <span>
          {total === 1 ? m.poll.voteSingular : format(m.poll.votesTotal, { count: String(total) })}
        </span>
        {poll.multipleChoice && <span>{m.poll.multipleTag}</span>}
        {poll.anonymous && <span>{m.poll.anonymousTag}</span>}
        <span>
          {open
            ? format(m.poll.closesIn, { remaining: remainingText(m, poll.closesAt, now) })
            : m.poll.closed}
        </span>
      </p>
    </section>
  );
}

/**
 * The current time, refreshed while the poll is open: every second in its last hour, when
 * seconds are shown, and every minute before that.
 */
function useNow(poll: Poll): number {
  const [now, setNow] = useState(() => Date.now());
  const open = pollOpen(poll, now);
  const lastHour = Date.parse(poll.closesAt) - now < 3_600_000;
  useEffect(() => {
    if (!open) {
      return;
    }
    const timer = setInterval(
      () => {
        setNow(Date.now());
      },
      lastHour ? 1000 : 60_000,
    );
    return () => {
      clearInterval(timer);
    };
  }, [open, lastHour]);
  return now;
}
