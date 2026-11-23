import { SourceScope } from "@/api/deployments";
import type { AccountEntry } from "@/api/accounts";
import { Avatar } from "@/features/communities/Avatar";
import { accountHost, type AccountSummary } from "./useAccountSummary";

/** An account's picture, read from its own home, or its deployment's initials while signed out. */
export function AccountPicture({
  entry,
  summary,
  size,
}: {
  entry: AccountEntry;
  summary: AccountSummary;
  size: "xs" | "sm" | "md" | "lg";
}) {
  const name = summary.name ?? accountHost(entry);
  return summary.home === null ? (
    <Avatar name={name} size={size} />
  ) : (
    <SourceScope source={summary.home}>
      <Avatar name={name} iconId={summary.icon} size={size} />
    </SourceScope>
  );
}
