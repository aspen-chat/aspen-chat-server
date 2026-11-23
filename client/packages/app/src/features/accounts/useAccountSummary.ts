import { useMemo, useSyncExternalStore } from "react";
import { useAllSources, type AccountEntry } from "@/api/accounts";
import { useAcross, type Source } from "@/api/everywhere";
import { displayNameOf, handleOf } from "@/features/users/profile";

export interface AccountSummary {
  /** The account's home, once it is signed in and connected. */
  readonly home: Source | null;
  readonly signedIn: boolean;
  readonly name: string | null;
  readonly handle: string | null;
  readonly icon: string | null;
  readonly unread: boolean;
  readonly tags: number;
}

const HOST = (serverUrl: string) => new URL(serverUrl).host;

/** What the account switcher says of an account, from whichever of its stores are running. */
export function useAccountSummary({ account, client }: AccountEntry): AccountSummary {
  const session = useSyncExternalStore(client.subscribe, () => client.session);
  const all = useAllSources();
  const sources = useMemo(() => all.filter((s) => s.accountId === account.id), [all, account.id]);
  return useAcross(sources, ["me", "unread"], (held) => {
    const home = held.find((source) => source.domain === null) ?? null;
    const me = home?.sync.store.me() ?? null;
    const hidden = home?.sync.store.doNotDisturb() === true;
    let unread = false;
    let tags = 0;
    for (const { sync } of held) {
      for (const place of sync.store.unreadPlaces()) {
        unread = true;
        tags += sync.store.placeMentions(place);
      }
    }
    return {
      home,
      signedIn: session !== null,
      name: me === null ? null : displayNameOf(me),
      handle: me === null ? null : `${handleOf(me)} · ${HOST(account.serverUrl)}`,
      icon: me?.icon ?? null,
      unread: !hidden && unread,
      tags: hidden ? 0 : tags,
    };
  });
}

export function accountHost(entry: AccountEntry): string {
  return HOST(entry.account.serverUrl);
}
