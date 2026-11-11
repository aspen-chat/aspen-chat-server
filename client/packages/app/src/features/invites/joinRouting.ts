import type { AspenLink } from "@/features/qr/aspenLinks";

/** What the add-a-server form starts filled in with (`OtherServerForm`). */
export interface OtherServerPrefill {
  domain: string;
  registrationInvite: string;
}

/** What the join form does with a link it was handed. */
export type JoinRouting =
  /** An invite of a community, or a registration invite of the deployment in use: open it. */
  | { action: "open"; link: Extract<AspenLink, { kind: "invite" | "registration" }> }
  /** A registration invite of another deployment: sign in there with the home account. */
  | { action: "otherServer"; prefill: OtherServerPrefill }
  | { action: "refuse"; reason: "notAnInvite" }
  /** As `otherServer`, where the home's accounts cannot sign in at other deployments. */
  | { action: "refuse"; reason: "registrationElsewhere"; domain: string };

/**
 * Decides what the join form does with a scanned or pasted link. A registration invite of
 * another deployment is not for making an account there, since the reader has one at home: it is
 * what that deployment asks of a first arrival, so it goes to the add-a-server form with the
 * deployment and the invite filled in. `otherServers` is whether the home takes part in
 * federation, without which that form is not offered and the invite is refused, naming its
 * deployment.
 */
export function routeJoinLink(link: AspenLink, otherServers: boolean): JoinRouting {
  if (link.kind === "invite") {
    return { action: "open", link };
  }
  if (link.kind !== "registration") {
    return { action: "refuse", reason: "notAnInvite" };
  }
  if (link.domain === null) {
    return { action: "open", link };
  }
  return otherServers
    ? {
        action: "otherServer",
        prefill: { domain: link.domain, registrationInvite: link.code },
      }
    : { action: "refuse", reason: "registrationElsewhere", domain: link.domain };
}
