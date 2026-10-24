/**
 * Signing in through the deployment's OpenID Connect identity providers, from the client's side
 * (`docs/architecture/sign-in-and-security/identity-providers.md` at the repository's root).
 *
 * The client starts a flow with a PKCE challenge of its own and sends the browser to the
 * provider. The provider returns it to the server, which sends it on to the flow's return
 * address with the flow's id and a one-time return code: the web client's `/sign-in/provider`
 * page, in the fragment (`parseProviderFragment`), or an app's loopback or `aspen:` address, in
 * the query, where a passkey hand-off's listener takes it (`parseHandoffReturn`). The client
 * claims the outcome with its verifier and that code.
 */

import type { components } from "./generated/openapi";
import type { Session } from "./session";

type Schemas = components["schemas"];

export type IdentityProviderPurpose = Schemas["IdentityProviderPurpose"];
export type OfferedIdentityProvider = Schemas["OfferedIdentityProvider"];
export type AccountOffer = Schemas["AccountOffer"];
export type LinkedIdentity = Schemas["LinkedIdentity"];

/** What making the account an `accountRequired` claim offered takes: the flow and its verifier. */
export interface PendingAccount {
  flow: string;
  verifier: string;
}

/** How a claimed flow ended. A `signedIn` session is already stored. */
export type IdentityProviderOutcome =
  | { outcome: "signedIn"; session: Session }
  | { outcome: "secondFactorRequired"; ticket: string; methods: Schemas["SecondFactorMethod"][] }
  | { outcome: "accountRequired"; offer: AccountOffer; account: PendingAccount }
  | { outcome: "linked"; identity: LinkedIdentity }
  | { outcome: "reauthenticated"; verifiedUntil: string };

/** What the browser brought back to the web client's return page. */
export interface ProviderReturn {
  flow: string;
  code: string;
}

/**
 * Reads the web client's return page's fragment (`#flow={id}&code={code}`); `null` when either
 * is missing, as on a page opened by hand.
 */
export function parseProviderFragment(hash: string): ProviderReturn | null {
  const params = new URLSearchParams(hash.replace(/^#/, ""));
  const flow = params.get("flow");
  const code = params.get("code");
  if (flow === null || flow === "" || code === null || code === "") {
    return null;
  }
  return { flow, code };
}

/**
 * The providers `GET /auth/methods` offers, in its order; none from a deployment whose server
 * predates them and so leaves the field out.
 */
export function offeredProviders(methods: Schemas["AuthMethods"]): OfferedIdentityProvider[] {
  return (methods as Partial<Schemas["AuthMethods"]>).identityProviders ?? [];
}
