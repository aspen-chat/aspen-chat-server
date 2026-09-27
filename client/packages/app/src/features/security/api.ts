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
