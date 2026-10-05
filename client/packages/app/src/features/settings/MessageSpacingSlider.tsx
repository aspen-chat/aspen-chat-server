import { MESSAGE_SPACING, MESSAGE_SPACINGS, type MessageSpacing } from "@aspen/protocol";
import type { CSSProperties } from "react";
import { usePreference, useSync } from "@/api/hooks";
import { StepSlider } from "@/features/layout/StepSlider";
import { useMessages } from "@/i18n/context";

/**
 * How far apart the lines and paragraphs of messages are drawn (`MESSAGE_SPACING`), kept with
 * the account, with two short paragraphs at the spacing under the thumb.
 */
export function MessageSpacingSlider() {
  const m = useMessages();
  const sync = useSync();
  const kept = usePreference(MESSAGE_SPACING);
  const names: Record<MessageSpacing, string> = {
    1: m.settings.messageSpacingNormal,
    1.2: m.settings.messageSpacingWide,
    1.4: m.settings.messageSpacingWider,
  };
  return (
    <StepSlider
      label={m.settings.messageSpacing}
      values={MESSAGE_SPACINGS}
      value={kept}
      describe={(spacing) => names[spacing]}
      onChoose={(spacing) => {
        void sync.preferences.set(MESSAGE_SPACING, spacing);
      }}
      preview={(spacing) => (
        <div
          aria-hidden="true"
          className="message-text message-body rounded-md border border-line px-2 py-1"
          style={{ "--message-line-scale": spacing } as CSSProperties}
        >
          <p>{m.settings.messageSpacingSample}</p>
          <p>{m.settings.messageSpacingSampleParagraph}</p>
        </div>
      )}
      hint={m.settings.messageSpacingHint}
    />
  );
}
