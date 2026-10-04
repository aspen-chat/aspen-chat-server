import { useNickname, useUser, useUserLoading } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useNameColor } from "@/features/users/nameColor";
import { displayNameOf, handleOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";

/**
 * A person's name as text: what they are called (or their handle, with `handle`) once their
 * record is here, a word-sized skeleton while it is on its way, and "Unknown user" only once
 * the server says there is no such person. Where `community` names the community it is shown
 * in, they are called by the nickname they chose there, if any. A name is drawn in its colour: a
 * deployment role's anywhere, and a community role's in its community.
 */
export function PersonName({
  id,
  handle = false,
  width = "w-20",
  community,
}: {
  id: string | undefined;
  handle?: boolean;
  /** How wide the skeleton is, a guess at the name's length. */
  width?: string;
  /** The community the name is shown in, if any. */
  community?: string | null | undefined;
}) {
  const m = useMessages();
  const user = useUser(id);
  const loading = useUserLoading(id);
  const color = useNameColor(id, community);
  const nickname = useNickname(community, id);
  if (user !== undefined) {
    const text = handle ? handleOf(user) : (nickname ?? displayNameOf(user));
    return color === undefined ? <>{text}</> : <span style={{ color }}>{text}</span>;
  }
  if (loading) {
    return (
      <>
        <LoadingLabel />
        <Skeleton inline className={width} />
      </>
    );
  }
  return <>{m.unknownUser}</>;
}

/** A person's picture, or a round skeleton its size while their record is on its way. */
export function PersonAvatar({
  id,
  size = "md",
}: {
  id: string | undefined;
  size?: "sm" | "md" | "lg";
}) {
  const user = useUser(id);
  const loading = useUserLoading(id);
  if (user === undefined && loading) {
    const dimensions = size === "lg" ? "h-12 w-12" : size === "md" ? "h-9 w-9" : "h-6 w-6";
    return <Skeleton className={`${dimensions} shrink-0 rounded-full`} />;
  }
  return (
    <Avatar name={user === undefined ? "?" : displayNameOf(user)} iconId={user?.icon} size={size} />
  );
}
