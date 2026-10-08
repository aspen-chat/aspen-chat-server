import { BellRingingIcon, BellSimpleIcon } from "@phosphor-icons/react";
import { ToggleButton } from "react-aria-components";
import { useFollowing, useSync } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";

/**
 * Follows the thread, so every reply in it tells the reader whatever its channel's
 * notifications, or stops following it. Taking part in a thread follows it too.
 */
export function FollowButton({ threadId }: { threadId: string }) {
  const m = useMessages();
  const sync = useSync();
  const following = useFollowing(threadId);
  const label = following ? m.threads.unfollow : m.threads.follow;
  return (
    <Tooltip text={label}>
      <ToggleButton
        aria-label={label}
        aria-description={m.threads.followHint}
        isSelected={following}
        onChange={(follow) => {
          void sync.setFollowing(threadId, follow).catch(() => undefined);
        }}
        className="tap-target rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 selected:text-accent"
      >
        {following ? (
          <BellRingingIcon size={18} weight="fill" aria-hidden="true" />
        ) : (
          <BellSimpleIcon size={18} aria-hidden="true" />
        )}
      </ToggleButton>
    </Tooltip>
  );
}
