import type { MessageKind } from "@aspen/protocol";

/** What a message's kind lets a reader do with it, whoever they are and whatever they may do. */
export interface KindActions {
  /** Whether a thread may be started on it. */
  readonly threadable: boolean;
  /** Whether it may be reported. */
  readonly reportable: boolean;
}

/**
 * Every kind the generated `MessageKind` has, so a kind it gains does not compile until what it
 * allows is chosen. A record of something that happened (a poll closing, a call, a join, a
 * community's growth) and a thread's echo start no thread; a call's record and the notices of
 * who joined are no one's to report.
 */
const KIND_ACTIONS: Readonly<Record<MessageKind, KindActions>> = {
  standard: { threadable: true, reportable: true },
  poll: { threadable: true, reportable: true },
  command: { threadable: true, reportable: true },
  warning: { threadable: true, reportable: true },
  pollClosed: { threadable: false, reportable: true },
  threadEcho: { threadable: false, reportable: true },
  call: { threadable: false, reportable: false },
  missedCall: { threadable: false, reportable: false },
  memberJoined: { threadable: false, reportable: false },
  communityGrew: { threadable: false, reportable: false },
};

/** What a kind this build's types lack allows (a newer deployment may send one): nothing. */
const UNKNOWN_KIND: KindActions = { threadable: false, reportable: false };

export function kindActions(kind: MessageKind): KindActions {
  return (KIND_ACTIONS as Readonly<Partial<Record<string, KindActions>>>)[kind] ?? UNKNOWN_KIND;
}
