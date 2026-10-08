import { useCustomEmoji } from "@/api/hooks";
import { emojiIdOf } from "@/features/emoji/customEmoji";
import { useMessages } from "@/i18n/context";

/** A reaction's emoji as spoken: the glyph, or a custom emoji's `:name:`. */
export function useEmojiName(communityId: string | null, emoji: string): string {
  const m = useMessages();
  const custom = useCustomEmoji(communityId ?? "");
  const id = emojiIdOf(emoji);
  if (id === null) {
    return emoji;
  }
  const name = custom.find((e) => e.id === id)?.name;
  return name === undefined ? m.emoji.unknown : `:${name}:`;
}
