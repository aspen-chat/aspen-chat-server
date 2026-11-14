import { createContext, useContext, useEffect, useRef } from "react";

/** What a message box does with what is dropped on its pane. */
export type DropReceiver = (data: DataTransfer) => void;

/** How a message box inside a `DropZone` offers it a receiver, or takes it back with `null`. */
export const DropZoneContext = createContext<((receiver: DropReceiver | null) => void) | null>(
  null,
);

/**
 * Offers `receiver` what is dropped on the nearest `DropZone`, while it is mounted and not
 * `null`.
 */
export function useDropReceiver(receiver: DropReceiver | null): void {
  const register = useContext(DropZoneContext);
  const latest = useRef(receiver);
  useEffect(() => {
    latest.current = receiver;
  });
  const on = receiver !== null;
  useEffect(() => {
    if (register === null || !on) {
      return;
    }
    register((data) => {
      latest.current?.(data);
    });
    return () => {
      register(null);
    };
  }, [register, on]);
}
