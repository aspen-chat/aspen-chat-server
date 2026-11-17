import { EMOTICON_EMOJI } from "@aspen/protocol";
import { usePreference, useSync } from "@/api/hooks";
import { planeClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";

/** How the message box treats what the user writes: whether emoticons become emoji. */
export function MessagesSection() {
  const m = useMessages();
  const sync = useSync();
  const emoticons = usePreference(EMOTICON_EMOJI);
  return (
    <section aria-labelledby="settings-messages" className={planeClass}>
      <h3 id="settings-messages" className="text-lg font-semibold text-ink-muted">
        {m.settings.messages}
      </h3>
      <ChoiceCheckbox
        isSelected={emoticons}
        onChange={(selected) => {
          void sync.preferences.set(EMOTICON_EMOJI, selected);
        }}
        label={m.settings.emoticonEmoji}
        hint={m.settings.emoticonEmojiHint}
      />
    </section>
  );
}
