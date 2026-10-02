import Picker, { Categories, EmojiStyle, SkinTonePickerLocation, Theme } from "emoji-picker-react";
import { useEffect, useMemo, useState } from "react";
import { useCustomEmoji, useIcons } from "@/api/hooks";
import { referenceOf } from "@/features/emoji/customEmoji";
import { loadEmojiData, useEmojiLanguage, type EmojiData } from "@/features/emoji/emojiData";
import { useMediaQuery } from "@/features/layout/useMediaQuery";
import { useMessages } from "@/i18n/context";

/**
 * The picker's width, for its 40px emoji. The library lays emoji out itself, as many to a row
 * as the whole width holds, and then indents each row by its 10px category padding, so a width
 * that is a whole number of emoji would push the last one past the edge. A whole number of
 * emoji and 20px, the padding on both sides, fits exactly: eight across, or seven on a phone
 * narrower than the eight-across picker and the popover's margins.
 */
const WIDE = 8 * 40 + 20;
const NARROW = 7 * 40 + 20;
const FITS_WIDE = `(min-width: ${String(WIDE + 16)}px)`;

/**
 * The full emoji picker: search, categories, recently used, and skin tones. Renders the
 * platform's own emoji glyphs, so nothing is fetched from anywhere. Loaded lazily by
 * `ReactionPicker`; its colours come from the `emoji-picker` rules in `styles.css`.
 *
 * The emoji keep the library's own 40px size: it places them by a size it measures once they
 * render and assumes 40px until then, so any other size would lay the first rows out at the
 * wrong spacing, past the edge, for as long as that takes.
 */
export default function EmojiPicker({
  onPick,
  communityId,
}: {
  /** Called with the emoji, or a custom emoji's reference, as a message or reaction names it. */
  onPick: (emoji: string) => void;
  /** The community whose own emoji the picker offers too; none in a DM. */
  communityId: string | null;
}) {
  const m = useMessages();
  const wide = useMediaQuery(FITS_WIDE);
  const language = useEmojiLanguage();
  // The names and headings in the reader's language, with the community's own section called
  // what the app calls it; until they come, the room they will take.
  const [loaded, setLoaded] = useState<{ language: string; data: EmojiData } | null>(null);
  useEffect(() => {
    let live = true;
    void loadEmojiData(language).then((data) => {
      if (live) {
        setLoaded({ language, data });
      }
    });
    return () => {
      live = false;
    };
  }, [language]);
  const emojiData = useMemo(
    () =>
      loaded?.language !== language
        ? null
        : {
            ...loaded.data,
            categories: {
              ...loaded.data.categories,
              custom: { category: Categories.CUSTOM, name: m.emoji.customCategory },
            },
          },
    [loaded, language, m.emoji.customCategory],
  );
  const custom = useCustomEmoji(communityId ?? "");
  const iconIds = useMemo(() => custom.map((e) => e.icon), [custom]);
  const icons = useIcons(iconIds);
  // The library takes each custom emoji with its picture's address; one whose picture has not
  // arrived is left out until it has.
  const customEmojis = useMemo(
    () =>
      custom.flatMap((e) => {
        const url = icons.get(e.icon)?.downloadUrl;
        return url === undefined ? [] : [{ id: e.id, names: [e.name], imgUrl: url }];
      }),
    [custom, icons],
  );
  if (emojiData === null) {
    return (
      <div
        className="flex items-center justify-center text-sm text-ink-muted"
        style={{ width: wide ? WIDE : NARROW, height: 384 }}
      >
        {m.loading}
      </div>
    );
  }
  return (
    <Picker
      emojiData={emojiData}
      onEmojiClick={(picked) => {
        onPick(picked.isCustom ? referenceOf(picked.unified) : picked.emoji);
      }}
      customEmojis={customEmojis}
      emojiStyle={EmojiStyle.NATIVE}
      theme={Theme.AUTO}
      lazyLoadEmojis
      skinTonePickerLocation={SkinTonePickerLocation.SEARCH}
      previewConfig={{ showPreview: false }}
      searchPlaceholder={m.emojiSearch}
      width={wide ? WIDE : NARROW}
      height={384}
      className="emoji-picker"
    />
  );
}
