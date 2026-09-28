/**
 * A stretch of a message window: one message shown as it is, or consecutive messages by people
 * the reader blocked, which are collapsed into one row they may open.
 */
export type WindowPart =
  | { readonly kind: "message"; readonly id: string; readonly index: number }
  | { readonly kind: "blocked"; readonly ids: readonly string[]; readonly index: number };

/**
 * Splits `ids` into messages and runs of blocked ones, in order. `index` is where each part
 * starts in `ids`.
 */
export function windowParts(
  ids: readonly string[],
  isBlocked: (id: string) => boolean,
): WindowPart[] {
  const parts: WindowPart[] = [];
  let run: string[] = [];
  let runStart = 0;
  const closeRun = () => {
    if (run.length > 0) {
      parts.push({ kind: "blocked", ids: run, index: runStart });
      run = [];
    }
  };
  for (const [index, id] of ids.entries()) {
    if (isBlocked(id)) {
      if (run.length === 0) {
        runStart = index;
      }
      run.push(id);
    } else {
      closeRun();
      parts.push({ kind: "message", id, index });
    }
  }
  closeRun();
  return parts;
}
