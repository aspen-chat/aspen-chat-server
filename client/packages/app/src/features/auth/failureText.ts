import { ApiProblemError, DevicePuzzleRefusedError, UnreachableError } from "@aspen/protocol";
import { format, type Messages } from "@/i18n/messages";

/**
 * What a failed request says to the person who made it: a refusal in the server's own words, and
 * a server that never answered (or whose device puzzle this client gave up on) as unreachable,
 * named by its host, with what to do.
 */
export function failureText(e: unknown, m: Messages): string {
  if (e instanceof ApiProblemError) {
    return e.message;
  }
  if (e instanceof UnreachableError || e instanceof DevicePuzzleRefusedError) {
    return format(m.serverUnreachable, { server: new URL(e.server).host });
  }
  return String(e);
}
