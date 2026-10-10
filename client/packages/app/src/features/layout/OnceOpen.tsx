import { useContext, useState, type ReactNode } from "react";
import { OverlayTriggerStateContext, TooltipTriggerStateContext } from "react-aria-components";

/** Whether `value` has been true in any render so far. */
function useOnceTrue(value: boolean): boolean {
  const [seen, setSeen] = useState(value);
  if (value && !seen) {
    setSeen(true);
  }
  return seen || value;
}

/**
 * Builds an overlay (a popover, a modal, a sheet) from when it first opens, and not before. A
 * closed React Aria overlay draws nothing but still holds the state of one, over a hundred
 * hooks, which a list that gives each of its rows a profile card or a dialog pays for every
 * row. It stays built once it has opened, so it can move as it closes.
 *
 * Whether it is open is `isOpen`, for an overlay opened by state its owner keeps, or else the
 * state of the trigger it stands in (`DialogTrigger`, `MenuTrigger`).
 */
export function OnceOpen({ isOpen, children }: { isOpen?: boolean; children: ReactNode }) {
  const trigger = useContext(OverlayTriggerStateContext);
  return useOnceTrue(isOpen ?? trigger?.isOpen ?? false) ? children : null;
}

/** `OnceOpen` for the tooltip of the `TooltipTrigger` it stands in. */
export function TooltipOnceOpen({ children }: { children: ReactNode }) {
  const trigger = useContext(TooltipTriggerStateContext);
  return useOnceTrue(trigger?.isOpen ?? false) ? children : null;
}
