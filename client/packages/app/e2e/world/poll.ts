import { bob, general, lunchPoll, me, messageId, minutesAgo, pollQuestion } from "./fixtures";
import { type Publish, reply } from "./reply";

/**
 * Bob's poll, one per page so that what a spec writes in stays in that spec. Bob has written in
 * an answer already; the caller has written in none. Each change is published as the poll's
 * `update` event, as the server does.
 */
export function lunch(publish: Publish) {
  const poll = {
    id: lunchPoll,
    channelId: general,
    messageId: messageId(207),
    createdBy: bob,
    createdAt: minutesAgo(3),
    closesAt: minutesAgo(-60),
    closedAt: null,
    question: pollQuestion,
    options: [{ label: "Pizza" }, { label: "Sushi" }],
    multipleChoice: false,
    allowWriteIns: true,
    writeIns: [{ label: "Pancakes", writtenBy: bob }] as ({
      label: string;
      writtenBy: string;
    } | null)[],
    anonymous: false,
    results: [
      { count: 1, voters: [bob] },
      { count: 0, voters: [] },
      { count: 0, voters: [] },
    ] as { count: number; voters: string[] }[],
  };
  const changed = () => {
    publish({
      serverEvent: "poll",
      type: "update",
      id: poll.id,
      writeIns: poll.writeIns,
      results: poll.results,
    });
  };
  return {
    read: () => ({ data: poll, included: { pollVotes: [], ownWriteIns: [] } }),
    writeIn: (label: string) => {
      poll.writeIns.push({ label, writtenBy: me });
      poll.results.push({ count: 1, voters: [me] });
      changed();
      return reply({ option: poll.results.length - 1, poll }, 201);
    },
    remove: (option: number) => {
      poll.writeIns[option - poll.options.length] = null;
      poll.results[option] = { count: 0, voters: [] };
      changed();
      return reply(null, 204);
    },
  };
}
