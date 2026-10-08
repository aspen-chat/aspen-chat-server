import { emojiIdOf } from "@/features/emoji/customEmoji";

/** How many emoji the quick reactions offer. */
export const QUICK_REACTIONS = 5;

/**
 * What the quick reactions offer where the reader's own most used do not fill them, in the
 * canonical form the server stores reactions in, so one already among theirs is not offered
 * twice.
 */
export const DEFAULT_QUICK_REACTIONS: readonly string[] = [
  "\u{1F604}",
  "\u{2764}\u{FE0F}",
  "\u{1F44D}",
  "\u{1F44E}",
  "\u{1F62E}",
];

/**
 * The emoji the quick reactions offer: the reader's most used, as the server ranks them
 * (`frequent`), leaving out a custom emoji that is not among `usableCustom`'s ids (one whose
 * record has not come, or that has gone), then the defaults, until there are
 * `QUICK_REACTIONS`.
 */
export function quickReactions(
  frequent: readonly string[],
  usableCustom: ReadonlySet<string>,
): string[] {
  const chosen: string[] = [];
  for (const emoji of [...frequent, ...DEFAULT_QUICK_REACTIONS]) {
    if (chosen.length === QUICK_REACTIONS) {
      break;
    }
    const custom = emojiIdOf(emoji);
    if ((custom === null || usableCustom.has(custom)) && !chosen.includes(emoji)) {
      chosen.push(emoji);
    }
  }
  return chosen;
}
