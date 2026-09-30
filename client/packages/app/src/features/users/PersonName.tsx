import { useUser, useUserLoading } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { displayNameOf, handleOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";

/**
 * A person's name as text: what they are called (or their handle, with `handle`) once their
 * record is here, a word-sized skeleton while it is on its way, and "Unknown user" only once
 * the server says there is no such person.
 */
export function PersonName({
  id,
  handle = false,
  width = "w-20",
}: {
  id: string | undefined;
  handle?: boolean;
  /** How wide the skeleton is, a guess at the name's length. */
  width?: string;
}) {
  const m = useMessages();
  const user = useUser(id);
  const loading = useUserLoading(id);
  if (user !== undefined) {
    return <>{handle ? handleOf(user) : displayNameOf(user)}</>;
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
