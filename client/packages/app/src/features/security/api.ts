import {
  API_PREFIX,
  ApiProblemError,
  problemOf,
  unwrap,
  type AspenClient,
  type Schemas,
} from "@aspen/protocol";

/**
 * The caller's security settings. Every call addresses `@me`: nobody reads or changes anyone
 * else's.
 */
export type SecuritySettings = Schemas["SecuritySettings"];
export type TotpEnrollment = Schemas["TotpEnrollment"];

const me = { params: { path: { user: "@me" } } } as const;

export async function fetchSecurity(client: AspenClient): Promise<SecuritySettings> {
  return unwrap(await client.api.GET(`${API_PREFIX}/users/{user}/security`, me));
}

export async function beginTotp(client: AspenClient): Promise<TotpEnrollment> {
  return unwrap(await client.api.POST(`${API_PREFIX}/users/{user}/totp`, me));
}

/** Returns the new recovery codes when this turned two-factor sign-in on. */
export async function confirmTotp(client: AspenClient, code: string): Promise<string[] | null> {
  const added = unwrap(
    await client.api.POST(`${API_PREFIX}/users/{user}/totp/confirmation`, {
      ...me,
      body: { code },
    }),
  );
  return added.recoveryCodes ?? null;
}

export async function removeTotp(client: AspenClient): Promise<void> {
  const result = await client.api.DELETE(`${API_PREFIX}/users/{user}/totp`, me);
  if (result.error !== undefined) {
    throw new ApiProblemError(problemOf(result.error, result.response));
  }
}

export async function renamePasskey(
  client: AspenClient,
  passkey: string,
  name: string,
): Promise<void> {
  unwrap(
    await client.api.PATCH(`${API_PREFIX}/users/{user}/passkeys/{passkey}`, {
      params: { path: { user: "@me", passkey } },
      body: { name },
    }),
  );
}

export async function removePasskey(client: AspenClient, passkey: string): Promise<void> {
  const result = await client.api.DELETE(`${API_PREFIX}/users/{user}/passkeys/{passkey}`, {
    params: { path: { user: "@me", passkey } },
  });
  if (result.error !== undefined) {
    throw new ApiProblemError(problemOf(result.error, result.response));
  }
}

export async function regenerateRecoveryCodes(client: AspenClient): Promise<string[]> {
  return unwrap(await client.api.POST(`${API_PREFIX}/users/{user}/recovery-codes`, me)).codes;
}

/**
 * Replaces the password, or with `oldPassword` `null` sets the first one of an account that has
 * none (one made through an identity provider), which needs a recent verification instead. The
 * server signs out every other session of the account and keeps this one; an account with
 * two-factor sign-in also needs a recent verification.
 */
export async function changePassword(
  client: AspenClient,
  oldPassword: string | null,
  newPassword: string,
): Promise<void> {
  const result = await client.api.PUT(`${API_PREFIX}/users/{user}/password`, {
    ...me,
    body: oldPassword === null ? { newPassword } : { oldPassword, newPassword },
  });
  if (result.error !== undefined) {
    throw new ApiProblemError(problemOf(result.error, result.response));
  }
}

/**
 * Signs out everywhere else: the server ends every other sign-in of the account, and every
 * plugin capability URL, and keeps this one. Needs a recent verification.
 */
export async function endOtherSignIns(client: AspenClient): Promise<void> {
  const result = await client.api.DELETE(`${API_PREFIX}/users/{user}/sign-ins`, me);
  if (result.error !== undefined) {
    throw new ApiProblemError(problemOf(result.error, result.response));
  }
}

/**
 * Unlinks an identity provider from the account. Needs a recent verification; refused with
 * `lastSignInMethod` when it is the account's only way in.
 */
export async function unlinkIdentity(client: AspenClient, identity: string): Promise<void> {
  const result = await client.api.DELETE(`${API_PREFIX}/users/{user}/identities/{identity}`, {
    params: { path: { user: "@me", identity } },
  });
  if (result.error !== undefined) {
    throw new ApiProblemError(problemOf(result.error, result.response));
  }
}

/** Whether the account has a password; a server that predates accounts without one says nothing. */
export function hasPassword(settings: SecuritySettings): boolean {
  return (settings as Partial<SecuritySettings>).hasPassword ?? true;
}

/** The identity providers linked to the account; none from a server that predates them. */
export function linkedIdentities(settings: SecuritySettings): Schemas["LinkedIdentity"][] {
  return (settings as Partial<SecuritySettings>).identities ?? [];
}
