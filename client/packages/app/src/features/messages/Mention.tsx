import { useContext } from "react";
import { Button } from "react-aria-components";
import { useRoles, useUser } from "@/api/hooks";
import { MentionContext } from "@/features/messages/mentionContext";
import type { MentionKind } from "@/features/messages/remarkMentions";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { useNameColor, useRoleColor } from "@/features/users/nameColor";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { PersonName } from "@/features/users/PersonName";

const chipClass =
  "rounded bg-accent-soft px-0.5 font-medium text-accent-strong outline-none " +
  "hover:underline focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * A tag in a message body. One that counts, as the server decided, is a chip: a person's
 * opens their card. One its author could not make is plain text naming whom it would have
 * tagged, as the server also leaves it.
 */
export function Mention({ kind, id, text }: { kind: MentionKind; id: string; text: string }) {
  const context = useContext(MentionContext);
  const counts =
    context !== null &&
    (kind === "everyone"
      ? context.mentions.everyone
      : kind === "user"
        ? context.mentions.users.includes(id)
        : context.mentions.roles.includes(id));
  if (kind === "user") {
    return <UserMention id={id} chip={counts} />;
  }
  if (kind === "role") {
    return <RoleMention id={id} communityId={context?.communityId ?? null} chip={counts} />;
  }
  return counts ? <span className={chipClass}>{text}</span> : <>{text}</>;
}

/**
 * A person named in text: "@" and their name, as a chip that opens their card where `chip`
 * says they count, drawn in their name's colour where `communityId` is (by default, the
 * community of the message around it), and plain text otherwise.
 */
export function UserMention({
  id,
  chip,
  communityId,
}: {
  id: string;
  chip: boolean;
  communityId?: string | null;
}) {
  const m = useMessages();
  const context = useContext(MentionContext);
  const user = useUser(id);
  const color = useNameColor(id, communityId === undefined ? context?.communityId : communityId);
  if (user === undefined) {
    return (
      <>
        @<PersonName id={id} />
      </>
    );
  }
  const name = `@${displayNameOf(user)}`;
  if (!chip) {
    return <>{name}</>;
  }
  return (
    <ProfilePopover user={user}>
      <Button
        aria-label={format(m.profile.show, { name: displayNameOf(user) })}
        className={chipClass + " inline"}
        style={{ color }}
      >
        {name}
      </Button>
    </ProfilePopover>
  );
}

function RoleMention({
  id,
  communityId,
  chip,
}: {
  id: string;
  communityId: string | null;
  chip: boolean;
}) {
  const m = useMessages();
  const role = useRoles(communityId ?? "").find((r) => r.id === id);
  const color = useRoleColor(role);
  const name = `@${role?.name ?? m.unknownRole}`;
  return chip ? (
    <span className={chipClass} style={{ color }}>
      {name}
    </span>
  ) : (
    <>{name}</>
  );
}
