import { ApiProblemError } from "@aspen/protocol";

/** The text to show for a failed request: a Problem's localized message, or the error as text. */
export function problemText(e: unknown): string {
  return e instanceof ApiProblemError ? e.message : String(e);
}
