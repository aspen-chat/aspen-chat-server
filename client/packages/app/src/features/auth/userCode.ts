/**
 * The letters of a sign-in code's user code: RFC 8628's base-20 alphabet, as the server's
 * `device_link::CODE_ALPHABET`.
 */
const ALPHABET = "BCDFGHJKLMNPQRSTVWXZ";
/** Letters in a user code, as the server's `device_link::CODE_LENGTH`. */
export const USER_CODE_LENGTH = 8;
const USER_CODE = new RegExp(`^[${ALPHABET}]{${String(USER_CODE_LENGTH)}}$`, "u");

/**
 * A typed user code as the server reads it: in compatibility form (full-width letters as the
 * letters they are), hyphens and whitespace dropped, in capitals. `null` for anything that
 * cannot be one, which is not worth sending: the server counts every code it is sent that
 * matches nothing against the person typing.
 */
export function normalizeUserCode(typed: string): string | null {
  const code = typed
    .normalize("NFKC")
    .replace(/[\s-]/gu, "")
    .replace(/[a-z]/gu, (letter) => letter.toUpperCase());
  return USER_CODE.test(code) ? code : null;
}
