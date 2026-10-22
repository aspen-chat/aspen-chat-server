import type { Reply } from "@aspen/protocol";
import { ArrowBendUpLeftIcon, AtIcon, XIcon } from "@phosphor-icons/react";
import { Button, ToggleButton } from "react-aria-components";
import { useMe, useMessageOnDemand, useNickname, useUser } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { PersonName } from "@/features/users/PersonName";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { formatNodes } from "@/i18n/formatNodes";

const barButtonClass =
  "flex shrink-0 items-center gap-1 rounded-md px-1.5 py-1 text-xs font-medium text-ink-muted outline-none " +
  "hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 tap-target";

/**
 * Over the message box while what is written there replies to a message: whom it replies to,
 * whether it tags them (on unless turned off, and offered only where the reader may tag members
 * and the message is someone else's), and a way to stop replying, as Escape in the box does too.
 * A message deleted meanwhile is said to be, since sending would be refused.
 */
export function ReplyBar({
  reply,
  community,
  mayTag,
  onTag,
  onCancel,
}: {
  reply: Reply;
  /** The community the channel is in, whose nicknames name people; `null` in a DM. */
  community: string | null;
  /** Whether the reader may tag members where they are writing. */
  mayTag: boolean;
  onTag: (tag: boolean) => void;
  onCancel: () => void;
}) {
  const m = useMessages();
  const me = useMe();
  const { message: original, missing } = useMessageOnDemand(reply.to, true);
  const author = useUser(original?.author);
  const nickname = useNickname(community, original?.author);
  const name = author === undefined ? m.unknownUser : (nickname ?? displayNameOf(author));
  const taggable = mayTag && original !== undefined && original.author !== me?.id;
  return (
    <div className="flex items-center gap-2 rounded-md bg-surface-sunken ps-2 text-sm text-ink-muted">
      <ArrowBendUpLeftIcon size={16} aria-hidden="true" className="shrink-0 rtl:-scale-x-100" />
      <p className="min-w-0 flex-1 truncate">
        {missing
          ? m.replies.targetDeleted
          : formatNodes(m.replies.replyingToName, {
              name: (
                <span className="font-medium">
                  <PersonName id={original?.author} community={community} />
                </span>
              ),
            })}
      </p>
      {taggable && (
        <Tooltip text={format(m.replies.tagAuthor, { name })}>
          <ToggleButton
            isSelected={reply.tag}
            onChange={onTag}
            aria-label={format(m.replies.tagAuthor, { name })}
            className={barButtonClass + " selected:text-accent"}
          >
            <AtIcon size={14} aria-hidden="true" />
            <span aria-hidden="true">{reply.tag ? m.replies.tagOn : m.replies.tagOff}</span>
          </ToggleButton>
        </Tooltip>
      )}
      <Tooltip text={m.replies.cancel}>
        <Button aria-label={m.replies.cancel} onPress={onCancel} className={barButtonClass}>
          <XIcon size={14} aria-hidden="true" />
        </Button>
      </Tooltip>
    </div>
  );
}
