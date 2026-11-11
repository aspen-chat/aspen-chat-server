import type { ModerationEntry } from "@aspen/protocol";

/** Who took a moderation log entry's action, as the server names them (`ModerationEntry.actorKind`). */
export type ActorKind = NonNullable<ModerationEntry["actorKind"]>;

/**
 * Who took an entry's action. The deployment says (`actorKind`); one of a version that does not
 * names no account both for an operator's command and for an account since gone, and only ever
 * logged one action from the terminal, purging evidence, which nothing else wrote.
 */
export function actorKindOf(
  entry: Pick<ModerationEntry, "actor" | "actorKind" | "action">,
): ActorKind {
  if (entry.actorKind != null) {
    return entry.actorKind;
  }
  if (entry.actor != null) {
    return "user";
  }
  return entry.action === "purgeAttachment" ? "operator" : "deletedAccount";
}
