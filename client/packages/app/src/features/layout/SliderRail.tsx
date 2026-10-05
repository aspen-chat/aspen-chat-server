import type { RefObject } from "react";
import { SliderThumb, SliderTrack } from "react-aria-components";

/** A slider's track and thumb, as every slider in the app draws them. */
export function SliderRail({ inputRef }: { inputRef?: RefObject<HTMLInputElement | null> }) {
  return (
    <SliderTrack className="relative h-6 w-full">
      <div className="forced-fill absolute top-1/2 h-1 w-full -translate-y-1/2 rounded-full bg-line" />
      <SliderThumb
        {...(inputRef === undefined ? {} : { inputRef })}
        className="top-1/2 h-4 w-4 rounded-full border border-line bg-accent outline-none dragging:bg-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50"
      />
    </SliderTrack>
  );
}
