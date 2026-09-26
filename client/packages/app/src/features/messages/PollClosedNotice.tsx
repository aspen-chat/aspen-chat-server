import { ChartBarIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { usePoll } from "@/api/hooks";
import { outcomeText } from "@/features/messages/poll";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The system message a closed poll leaves in its channel: which option won, or that it was a
 * tie or nobody voted, with a link back to the poll itself. The text is composed here from the
 * poll's final tally, so it reads in the reader's language.
 */
export function PollClosedNotice({
  pollId,
  communityId,
  channelId,
}: {
  pollId: string;
  communityId: string;
  channelId: string;
}) {
  const m = useMessages();
  const poll = usePoll(pollId);
  return (
    <div className="flex items-start gap-2 text-sm text-ink-muted">
      <ChartBarIcon size={18} aria-hidden="true" className="mt-0.5 shrink-0" />
      {poll === undefined ? (
        <span>{m.poll.unavailable}</span>
      ) : (
        <span>
          {format(m.poll.closedNotice, { question: poll.question })} {outcomeText(m, poll)}{" "}
          <Link
            to="/communities/$communityId/channels/$channelId/messages/$messageId"
            params={{ communityId, channelId, messageId: poll.messageId }}
            className="text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            {m.poll.showPoll}
          </Link>
        </span>
      )}
    </div>
  );
}
