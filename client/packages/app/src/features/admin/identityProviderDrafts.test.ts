import type { IdentityProviderSettings } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { createRequest, draftOf, NEW_PROVIDER, providerPatch } from "./identityProviderDrafts";

const saved: IdentityProviderSettings = {
  id: "0190f0a0-0000-7000-8000-0000000000aa",
  name: "Example Corp",
  issuer: "https://idp.example.org/realms/staff",
  clientId: "aspen",
  enabled: true,
  skipInvite: false,
  enforcesTwoFactor: false,
  trustEmail: true,
  createdAt: "2026-10-01T00:00:00Z",
  linkedAccounts: 3,
};

describe("identity provider drafts", () => {
  it("patch nothing when nothing changed, and never send the secret unasked", () => {
    expect(providerPatch(saved, draftOf(saved), false)).toEqual({});
    expect(providerPatch(saved, { ...draftOf(saved), clientSecret: "typed" }, false)).toEqual({});
    // Asked to replace it but left empty, the secret stays as it is.
    expect(providerPatch(saved, draftOf(saved), true)).toEqual({});
  });

  it("patch only what changed, trimmed", () => {
    expect(
      providerPatch(
        saved,
        {
          ...draftOf(saved),
          name: "  Example Staff ",
          enabled: false,
          clientSecret: "new secret",
        },
        true,
      ),
    ).toEqual({ name: "Example Staff", enabled: false, clientSecret: "new secret" });
  });

  it("never patch the issuer or client ID, which are fixed once added", () => {
    expect(
      providerPatch(
        saved,
        { ...draftOf(saved), issuer: "https://idp.attacker.example", clientId: "other" },
        false,
      ),
    ).toEqual({});
  });

  it("make a new provider with the server's defaults for its choices", () => {
    expect(
      createRequest({
        ...NEW_PROVIDER,
        name: " Example ",
        issuer: " https://idp.example.org ",
        clientId: "aspen ",
        clientSecret: " secret with spaces ",
      }),
    ).toEqual({
      name: "Example",
      issuer: "https://idp.example.org",
      clientId: "aspen",
      clientSecret: " secret with spaces ",
      enabled: true,
      skipInvite: false,
      enforcesTwoFactor: false,
      trustEmail: true,
    });
  });
});
