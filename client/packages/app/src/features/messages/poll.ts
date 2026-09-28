import type { Poll, PollOption } from "@aspen/protocol";
import { format, type Messages } from "@/i18n/messages";

/** The longest option or written-in answer the server accepts, in characters. */
export const MAX_OPTION_CHARS = 100;
/** The most answers voters may write in to one poll, removed ones included. */
export const MAX_WRITE_INS = 25;

/** One answer a poll offers, at the index votes name it by. */
export interface PollChoice {
  index: number;
  option: PollOption;
  /** Whether a voter wrote it in, rather than the poll's creator listing it. */
  writeIn: boolean;
  /** Who wrote it in, when the poll says: never on an anonymous poll. */
  writtenBy: string | null;
}

/**
 * Every answer a poll offers: its creator's options, then the standing write-ins. Write-ins
 * number on from the options, and a removed one keeps its index, so the indices can skip.
 */
export function pollChoices(poll: Poll): PollChoice[] {
  const listed = poll.options.map((option, index) => ({
    index,
    option,
    writeIn: false,
    writtenBy: null,
  }));
  const written = poll.writeIns.flatMap((w, i) =>
    w === null
      ? []
      : [
          {
            index: poll.options.length + i,
            option: { label: w.label },
            writeIn: true,
            writtenBy: w.writtenBy ?? null,
          },
        ],
  );
  return [...listed, ...written];
}

/** The answer at `index`, an option or a standing write-in. */
export function choiceAt(poll: Poll, index: number): PollOption | undefined {
  if (index < poll.options.length) {
    return poll.options[index];
  }
  const written = poll.writeIns[index - poll.options.length];
  return written == null ? undefined : { label: written.label };
}

/** How a closed poll came out, as its announcement describes it. */
export type PollOutcome =
  | { kind: "noVotes" }
  | { kind: "winner"; option: number; count: number }
  | { kind: "tie"; options: number[]; count: number };

/** The outcome of a poll from its tally: the option with the most votes, or every option tied for it. */
export function pollOutcome(poll: Poll): PollOutcome {
  const most = Math.max(0, ...poll.results.map((r) => r.count));
  if (most === 0) {
    return { kind: "noVotes" };
  }
  const leaders = poll.results.flatMap((r, i) => (r.count === most ? [i] : []));
  const [first] = leaders;
  return leaders.length === 1 && first !== undefined
    ? { kind: "winner", option: first, count: most }
    : { kind: "tie", options: leaders, count: most };
}

/** An option as prose: its emoji, when it has one, then its label. */
export function optionName(option: PollOption | undefined): string {
  if (option === undefined) {
    return "";
  }
  return option.emoji == null ? option.label : `${option.emoji} ${option.label}`;
}

/** Whether votes are still accepted at `now`. */
export function pollOpen(poll: Poll, now: number): boolean {
  return poll.closedAt == null && Date.parse(poll.closesAt) > now;
}

/** Whole days, hours, minutes, and seconds left until `closesAt`, all zero once it has passed. */
export function timeLeft(
  closesAt: string,
  now: number,
): { days: number; hours: number; minutes: number; seconds: number } {
  const total = Math.max(0, Math.floor((Date.parse(closesAt) - now) / 1000));
  return {
    days: Math.floor(total / 86_400),
    hours: Math.floor((total % 86_400) / 3600),
    minutes: Math.floor((total % 3600) / 60),
    seconds: total % 60,
  };
}

/** Share of the votes an option holds, as a percentage of the total cast, for the result bars. */
export function sharePercent(poll: Poll, option: number): number {
  const total = poll.results.reduce((sum, r) => sum + r.count, 0);
  const count = poll.results[option]?.count ?? 0;
  return total === 0 ? 0 : Math.round((count / total) * 100);
}

/** The time left in the largest two units that apply. */
export function remainingText(m: Messages, closesAt: string, now: number): string {
  const left = timeLeft(closesAt, now);
  if (left.days > 0) {
    return format(m.poll.remaining.days, { n: String(left.days), h: String(left.hours) });
  }
  if (left.hours > 0) {
    return format(m.poll.remaining.hours, { n: String(left.hours), m: String(left.minutes) });
  }
  if (left.minutes > 0) {
    return format(m.poll.remaining.minutes, {
      n: String(left.minutes),
      s: String(left.seconds),
    });
  }
  return format(m.poll.remaining.seconds, { n: String(left.seconds) });
}

/** The announcement of a closed poll's result, from its final tally. */
export function outcomeText(m: Messages, poll: Poll): string {
  const outcome = pollOutcome(poll);
  switch (outcome.kind) {
    case "noVotes":
      return m.poll.noVotes;
    case "winner":
      return format(outcome.count === 1 ? m.poll.winnerSingular : m.poll.winner, {
        option: optionName(choiceAt(poll, outcome.option)),
        count: String(outcome.count),
      });
    case "tie":
      return format(outcome.count === 1 ? m.poll.tieSingular : m.poll.tie, {
        options: outcome.options.map((i) => optionName(choiceAt(poll, i))).join(", "),
        count: String(outcome.count),
      });
  }
}
