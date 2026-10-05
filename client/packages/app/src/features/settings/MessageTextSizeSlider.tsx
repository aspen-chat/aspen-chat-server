import { MESSAGE_TEXT_SIZE, MESSAGE_TEXT_SIZES } from "@aspen/protocol";
import type { CSSProperties } from "react";
import { usePreference, useSync } from "@/api/hooks";
import { messageTextScale } from "@/features/layout/messageTextSize";
import { StepSlider } from "@/features/layout/StepSlider";
import { useNumberFormat } from "@/i18n/format";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * How large messages are drawn (`MESSAGE_TEXT_SIZE`), kept with the account, with a line of
 * text at the size under the thumb.
 */
export function MessageTextSizeSlider() {
  const m = useMessages();
  const sync = useSync();
  const kept = usePreference(MESSAGE_TEXT_SIZE);
  const number = useNumberFormat();
  return (
    <StepSlider
      label={m.settings.messageTextSize}
      values={MESSAGE_TEXT_SIZES}
      value={kept}
      describe={(size) => format(m.settings.messageTextSizeValue, { size: number.format(size) })}
      onChoose={(size) => {
        void sync.preferences.set(MESSAGE_TEXT_SIZE, size);
      }}
      preview={(size) => (
        <p
          aria-hidden="true"
          className="message-text truncate"
          style={
            {
              "--message-text-scale": messageTextScale(size),
            } as CSSProperties
          }
        >
          {m.settings.messageTextSizeSample}
        </p>
      )}
      hint={m.settings.messageTextSizeHint}
    />
  );
}
