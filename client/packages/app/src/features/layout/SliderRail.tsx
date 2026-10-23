import type { RefObject } from "react";
import { SliderThumb, SliderTrack } from "react-aria-components";

/**
 * A slider's track and thumb, as every slider in the app draws them. With `filled`, the track
 * up to the thumb is drawn in the accent, for a slider whose value is how far along something is
 * (a sound's position, a volume).
 */
export function SliderRail({
  inputRef,
  filled = false,
}: {
  inputRef?: RefObject<HTMLInputElement | null>;
  filled?: boolean;
}) {
  return (
    <SliderTrack className="relative h-6 w-full">
      {({ state }) => (
        <>
          <div className="forced-fill absolute top-1/2 h-1 w-full -translate-y-1/2 rounded-full bg-line" />
          {filled && (
            <div
              className="forced-fill-highlight absolute top-1/2 h-1 -translate-y-1/2 rounded-full bg-accent"
              style={{ width: `${String(state.getThumbPercent(0) * 100)}%` }}
            />
          )}
          <SliderThumb
            {...(inputRef === undefined ? {} : { inputRef })}
            className="top-1/2 h-4 w-4 rounded-full border border-line bg-accent outline-none dragging:bg-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50"
          />
        </>
      )}
    </SliderTrack>
  );
}
