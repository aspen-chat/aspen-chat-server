import type { LinkPreview } from "@aspen/protocol";
import { PencilSimpleIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useMe, useMessage, useUser } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { Tooltip } from "@/features/layout/Tooltip";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { displayNameOf } from "@/features/users/profile";
import { DeleteMessageDialog } from "@/features/messages/DeleteMessageDialog";
import { MessageMedia } from "@/features/messages/Attachments";
import { imageUrls, isImageUrl, onlyImageLinks } from "@/features/messages/images";
import { Markdown } from "@/features/messages/Markdown";
import { VideoCard } from "@/features/messages/VideoCard";
import { playerSrc } from "@/features/messages/video";
import { MessageEditor } from "@/features/messages/MessageEditor";
import { PollCard } from "@/features/messages/PollCard";
import { PollClosedNotice } from "@/features/messages/PollClosedNotice";
import { ReactionChips, ReactionPicker } from "@/features/messages/Reactions";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const timeFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

const actionClass =
  "rounded px-2 py-0.5 text-xs text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

export function MessageItem({
  id,
  communityId,
  channelId,
  highlighted,
}: {
  id: string;
  communityId: string;
  channelId: string;
  highlighted: boolean;
}) {
  const m = useMessages();
  const message = useMessage(id);
  const author = useUser(message?.author);
  const me = useMe();
  const [editing, setEditing] = useState(false);
  if (message === undefined) {
    return null;
  }
  // Editing and deleting are offered only on the caller's own messages. The server accepts
  // either from anyone for now, but the controls should not invite it.
  const own = me !== null && me.id === message.author;
  // A poll message has no text of its own; its card is edited by voting, not by rewriting.
  const editable = own && message.kind === "standard";
  const linkedImages = imageUrls(message.content);
  // A link the client already shows as a picture needs no card from the server for the same URL.
  const previews = message.linkPreviews.filter((p) => !linkedImages.includes(p.url));
  // Links the server found to be images themselves join the message's pictures; the rest
  // are cards.
  const previewImages = previews.flatMap((p) =>
    isPictureOnly(p) && p.imageUrl != null ? [{ src: p.imageUrl, name: p.url }] : [],
  );
  const cards = previews.filter((p) => !isPictureOnly(p));
  // A message that is nothing but links to pictures shows the pictures alone.
  const pictureOnly = onlyImageLinks(
    message.content,
    (url) => isImageUrl(url) || previews.some((p) => p.url === url && isPictureOnly(p)),
  );
  if (message.kind === "pollClosed" && message.poll != null) {
    return (
      <article
        data-message-id={id}
        className={
          "flex gap-3 rounded-md px-2 py-1.5 " +
          (highlighted ? "bg-accent-soft" : "hover:bg-surface-hover/60")
        }
      >
        <div className="w-10 shrink-0" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <PollClosedNotice pollId={message.poll} communityId={communityId} channelId={channelId} />
        </div>
      </article>
    );
  }
  return (
    <article
      data-message-id={id}
      className={
        "group flex gap-3 rounded-md px-2 py-1.5 " +
        (highlighted ? "bg-accent-soft" : "hover:bg-surface-hover/60")
      }
    >
      <Avatar name={author === undefined ? "?" : displayNameOf(author)} iconId={author?.icon} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          {author === undefined ? (
            <span className="font-medium">{m.unknownUser}</span>
          ) : (
            <ProfilePopover user={author}>
              <Button
                aria-label={format(m.profile.show, { name: displayNameOf(author) })}
                className="rounded font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
              >
                {displayNameOf(author)}
              </Button>
            </ProfilePopover>
          )}
          <Link
            to="/communities/$communityId/channels/$channelId/messages/$messageId"
            params={{ communityId, channelId, messageId: id }}
            className="text-xs text-ink-faint outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
            title={m.linkToMessage}
          >
            <time dateTime={message.timestamp}>
              {timeFormat.format(new Date(message.timestamp))}
            </time>
          </Link>
          {!editing && (
            <span
              role="group"
              aria-label={m.messageActionsLabel}
              className="ml-auto flex gap-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100"
            >
              <ReactionPicker messageId={id} triggerClassName={actionClass} />
              {editable && (
                <Tooltip text={m.editMessage}>
                  <Button
                    onPress={() => {
                      setEditing(true);
                    }}
                    aria-label={m.editMessage}
                    className={actionClass}
                  >
                    <PencilSimpleIcon size={16} aria-hidden="true" />
                  </Button>
                </Tooltip>
              )}
              {own && (
                <DeleteMessageDialog
                  messageId={id}
                  triggerClassName={actionClass + " text-danger"}
                />
              )}
            </span>
          )}
        </div>
        {editing ? (
          <MessageEditor
            messageId={id}
            initial={message.content}
            onDone={() => {
              setEditing(false);
            }}
          />
        ) : (
          <div className="flex flex-wrap items-baseline gap-x-1">
            {!pictureOnly && <Markdown content={message.content} />}
            {message.editedAt != null && (
              <span
                className="text-xs text-ink-faint"
                title={format(m.editedAt, { date: timeFormat.format(new Date(message.editedAt)) })}
              >
                {m.edited}
              </span>
            )}
          </div>
        )}
        {message.kind === "poll" && message.poll != null && <PollCard pollId={message.poll} />}
        <MessageMedia
          attachmentIds={message.attachments}
          linkedImages={linkedImages}
          previewImages={previewImages}
        />
        {cards.map((preview) => (
          <LinkPreviewCard key={preview.url} preview={preview} />
        ))}
        <ReactionChips messageId={id} />
      </div>
    </article>
  );
}

/** A preview the server made from a link that is itself an image: a picture, no text. */
function isPictureOnly(preview: LinkPreview): boolean {
  return preview.imageUrl != null && preview.title == null && preview.description == null;
}

function LinkPreviewCard({ preview }: { preview: LinkPreview }) {
  const player = preview.video == null ? null : playerSrc(preview.video.src);
  if (preview.video != null && player !== null) {
    return <VideoCard preview={preview} player={player} />;
  }
  const title = preview.title ?? preview.siteName ?? preview.url;
  return (
    <a
      href={preview.url}
      target="_blank"
      rel="noreferrer"
      className="mt-1 flex max-w-lg gap-3 rounded-md border border-line bg-surface-raised p-2 text-sm hover:bg-surface-hover"
      style={
        preview.themeColor != null
          ? { borderLeftColor: preview.themeColor, borderLeftWidth: 3 }
          : undefined
      }
    >
      {preview.imageUrl != null && (
        <img src={preview.imageUrl} alt="" className="h-16 w-16 shrink-0 rounded object-cover" />
      )}
      <span className="min-w-0">
        <span className="block truncate font-medium text-accent">{title}</span>
        {preview.description != null && (
          <span className="mt-0.5 line-clamp-2 block text-ink-muted">{preview.description}</span>
        )}
      </span>
    </a>
  );
}
