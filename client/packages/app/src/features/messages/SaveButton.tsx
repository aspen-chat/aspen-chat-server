import { ApiProblemError } from "@aspen/protocol";
import { BookmarkSimpleIcon } from "@phosphor-icons/react";
import { useIsSaved, useSync } from "@/api/hooks";
import { IconAction } from "@/features/layout/IconAction";
import { toast } from "@/features/layout/toast";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** Saves the message to the reader's saved messages, or removes it from them. */
export function SaveButton({
  messageId,
  className,
  labelled = false,
  onPressed,
}: {
  messageId: string;
  className: string;
  /** Drawn with its name beside its icon, as a row of a list. */
  labelled?: boolean;
  /** Called as it is pressed, for a sheet offering it to close. */
  onPressed?: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const saved = useIsSaved(messageId);
  return (
    <IconAction
      label={saved ? m.saved.unsave : m.saved.save}
      labelled={labelled}
      onPress={() => {
        sync.setSaved(messageId, !saved).catch((error: unknown) => {
          // The server says why, such as having saved as many as it keeps.
          if (error instanceof ApiProblemError) {
            toast(format(m.saved.saveFailed, { problem: error.message }));
          }
        });
        onPressed?.();
      }}
      className={className}
      icon={
        <BookmarkSimpleIcon
          size={ACTION_ICON}
          weight={saved ? "fill" : "regular"}
          aria-hidden="true"
        />
      }
    />
  );
}
