import Picker, { EmojiStyle, SkinTonePickerLocation, Theme } from "emoji-picker-react";
import { useMessages } from "@/i18n/context";

/**
 * The full emoji picker: search, categories, recently used, and skin tones. Renders the
 * platform's own emoji glyphs, so nothing is fetched from anywhere. Loaded lazily by
 * `ReactionPicker`; its colours come from the `emoji-picker` rules in `styles.css`.
 */
export default function EmojiPicker({ onPick }: { onPick: (emoji: string) => void }) {
  const m = useMessages();
  return (
    <Picker
      onEmojiClick={(picked) => {
        onPick(picked.emoji);
      }}
      emojiStyle={EmojiStyle.NATIVE}
      theme={Theme.AUTO}
      lazyLoadEmojis
      skinTonePickerLocation={SkinTonePickerLocation.SEARCH}
      previewConfig={{ showPreview: false }}
      searchPlaceholder={m.emojiSearch}
      width={320}
      height={384}
      className="emoji-picker"
    />
  );
}
