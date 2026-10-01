import { createContext, useContext } from "react";

/**
 * How a row tells the message list that its height has just changed in the DOM, so the list
 * keeps the view still through the change in the same task, before anything else can run or
 * the frame is painted. A row calls it from a layout effect after rendering the change with
 * `flushSync`. Outside a message list (the pins list, search results) nothing moves, and it
 * does nothing.
 */
export const KeepStillContext = createContext<() => void>(() => undefined);

export function useKeepStill(): () => void {
  return useContext(KeepStillContext);
}
