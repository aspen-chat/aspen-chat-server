import { useRouter } from "@tanstack/react-router";
import { useCallback } from "react";
import { useAccountsHub } from "@/api/accounts";

/** Brings an account to the front, where it was left. */
export function useSwitchAccount(): (id: string) => void {
  const accounts = useAccountsHub();
  const router = useRouter();
  return useCallback(
    (id) => {
      accounts.bringToFront(id);
      router.history.push(accounts.routeOf(id) ?? "/");
    },
    [accounts, router],
  );
}
