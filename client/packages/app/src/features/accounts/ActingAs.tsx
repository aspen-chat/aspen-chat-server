import { useAccounts } from "@/api/accounts";
import { HomeScope } from "@/api/deployments";
import { useMe } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Who a message box posts as, over its corner, while several accounts are held: sending from the
 * wrong one cannot be taken back from the others' view. It takes no room in the box's layout.
 */
export function ActingAs() {
  const several = useAccounts().entries.length > 1;
  return several ? (
    <HomeScope>
      <ActingAsMe />
    </HomeScope>
  ) : null;
}

function ActingAsMe() {
  const m = useMessages().accounts;
  const me = useMe();
  if (me === null) {
    return null;
  }
  const name = displayNameOf(me);
  return (
    <span className="pointer-events-none absolute end-2 -top-3 z-10 flex max-w-[60%] items-center gap-1 rounded-full border border-line bg-surface-raised py-0.5 ps-0.5 pe-2 text-xs text-ink-muted">
      <Avatar name={name} iconId={me.icon} size="xs" />
      <span className="truncate">{format(m.actingAs, { name })}</span>
    </span>
  );
}
