import type {
  IdentityProviderCreateRequest,
  IdentityProviderSettings,
  IdentityProviderUpdateRequest,
} from "@aspen/protocol";

/** A provider's settings as its form edits them; the secret is only ever typed, never read. */
export interface Draft {
  name: string;
  issuer: string;
  clientId: string;
  /** A secret to send: required to add one, and on an edit only to replace the old. */
  clientSecret: string;
  enabled: boolean;
  skipInvite: boolean;
  enforcesTwoFactor: boolean;
  trustEmail: boolean;
}

/** As the server takes a provider whose request leaves the choices out. */
export const NEW_PROVIDER: Draft = {
  name: "",
  issuer: "",
  clientId: "",
  clientSecret: "",
  enabled: true,
  skipInvite: false,
  enforcesTwoFactor: false,
  trustEmail: false,
};

export function draftOf(provider: IdentityProviderSettings): Draft {
  return {
    name: provider.name,
    issuer: provider.issuer,
    clientId: provider.clientId,
    clientSecret: "",
    enabled: provider.enabled,
    skipInvite: provider.skipInvite,
    enforcesTwoFactor: provider.enforcesTwoFactor,
    trustEmail: provider.trustEmail,
  };
}

/**
 * What an edit changed, as a merge patch: a field left as it was is left out. The issuer and client
 * ID are fixed once a provider is added, so an edit never sends them.
 */
export function providerPatch(
  saved: IdentityProviderSettings,
  draft: Draft,
  replaceSecret: boolean,
): IdentityProviderUpdateRequest {
  const was = draftOf(saved);
  const patch: IdentityProviderUpdateRequest = {};
  if (draft.name.trim() !== was.name) {
    patch.name = draft.name.trim();
  }
  for (const key of ["enabled", "skipInvite", "enforcesTwoFactor", "trustEmail"] as const) {
    if (draft[key] !== was[key]) {
      patch[key] = draft[key];
    }
  }
  if (replaceSecret && draft.clientSecret !== "") {
    patch.clientSecret = draft.clientSecret;
  }
  return patch;
}

export function createRequest(draft: Draft): IdentityProviderCreateRequest {
  return {
    name: draft.name.trim(),
    issuer: draft.issuer.trim(),
    clientId: draft.clientId.trim(),
    clientSecret: draft.clientSecret,
    enabled: draft.enabled,
    skipInvite: draft.skipInvite,
    enforcesTwoFactor: draft.enforcesTwoFactor,
    trustEmail: draft.trustEmail,
  };
}
