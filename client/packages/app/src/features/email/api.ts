import {
  API_PREFIX,
  ApiProblemError,
  problemOf,
  unwrap,
  type AspenClient,
  type Schemas,
} from "@aspen/protocol";

/**
 * The caller's email address and what they receive there. Every call addresses `@me`: nobody
 * reads or changes anyone else's.
 */
export type EmailAccount = Schemas["EmailAccount"];
export type EmailAccountUpdate = Schemas["EmailAccountUpdateRequest"];
export type EmailPolicy = Schemas["EmailPolicy"];

const me = { params: { path: { user: "@me" } } } as const;

export async function fetchEmail(client: AspenClient): Promise<EmailAccount> {
  return unwrap(await client.api.GET(`${API_PREFIX}/users/{user}/email`, me));
}

export async function updateEmail(
  client: AspenClient,
  change: EmailAccountUpdate,
): Promise<EmailAccount> {
  return unwrap(
    await client.api.PATCH(`${API_PREFIX}/users/{user}/email`, { ...me, body: change }),
  );
}

/**
 * Gives the account an address or changes it, which the server mails a code to; a change needs
 * a recent verification (`reauthenticationRequired`).
 */
export async function setAddress(client: AspenClient, address: string): Promise<EmailAccount> {
  return unwrap(
    await client.api.PUT(`${API_PREFIX}/users/{user}/email/address`, { ...me, body: { address } }),
  );
}

export async function removeAddress(client: AspenClient): Promise<void> {
  const result = await client.api.DELETE(`${API_PREFIX}/users/{user}/email/address`, me);
  if (result.error !== undefined) {
    throw new ApiProblemError(problemOf(result.error, result.response));
  }
}

export async function resendCode(client: AspenClient): Promise<void> {
  const result = await client.api.POST(`${API_PREFIX}/users/{user}/email/verification-codes`, me);
  if (result.error !== undefined) {
    throw new ApiProblemError(problemOf(result.error, result.response));
  }
}

export async function verify(client: AspenClient, code: string): Promise<EmailAccount> {
  return unwrap(
    await client.api.POST(`${API_PREFIX}/users/{user}/email/verification`, {
      ...me,
      body: { code },
    }),
  );
}

/**
 * What the deployment does with email, from `GET /deployment`. A deployment that predates email
 * says nothing, and sends none.
 */
export async function fetchPolicy(client: AspenClient): Promise<EmailPolicy> {
  const profile = await client.deploymentProfile();
  return (
    profile.email ?? {
      available: false,
      required: false,
      verificationRequired: false,
      newsletter: false,
    }
  );
}
