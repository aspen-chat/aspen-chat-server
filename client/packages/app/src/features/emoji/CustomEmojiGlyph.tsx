import { useCustomEmoji, useIcon } from "@/api/hooks";
import { useMessages } from "@/i18n/context";

/**
 * One of a community's own emoji, drawn where a glyph would be: its picture at the text's
 * size, named for assistive technology and on hover by `:name:`. An id the community's list
 * does not hold (an emoji since removed, or another community's) is a marked placeholder.
 */
export function CustomEmojiGlyph({
  id,
  communityId,
  size = "text",
}: {
  id: string;
  /** The community whose list resolves the id; none in a DM, where no custom emoji is. */
  communityId: string | null;
  /** `text` for one among words, `large` for a reaction chip. */
  size?: "text" | "large";
}) {
  const m = useMessages();
  const emoji = useCustomEmoji(communityId ?? "").find((e) => e.id === id);
  const icon = useIcon(emoji?.icon);
  const box = size === "large" ? "h-[1.5em] w-[1.5em]" : "h-[1.375em] w-[1.375em]";
  if (emoji === undefined) {
    return (
      <span
        role="img"
        aria-label={m.emoji.unknown}
        title={m.emoji.unknown}
        className={`inline-block ${box} rounded border border-dashed border-line align-text-bottom`}
      />
    );
  }
  const name = `:${emoji.name}:`;
  if (icon === undefined) {
    return (
      <span
        role="img"
        aria-label={name}
        title={emoji.name}
        className={`inline-block ${box} rounded bg-surface-hover align-text-bottom`}
      />
    );
  }
  return (
    <img
      src={icon.downloadUrl}
      alt={name}
      title={emoji.name}
      draggable={false}
      className={`inline-block ${box} object-contain align-text-bottom`}
    />
  );
}
