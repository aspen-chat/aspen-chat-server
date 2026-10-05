import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Label, Slider, SliderOutput, Text } from "react-aria-components";
import { SliderRail } from "@/features/layout/SliderRail";

/**
 * A slider over a few set values, unevenly spaced (text sizes, zoom factors), moving from one to
 * the next. The slider's own value is the position among them, and `describe` words each for
 * the eye and for assistive technology alike. The choice is made when the thumb is let go (or with each key
 * press), not as it moves, since what it changes may move the slider itself.
 */
export function StepSlider<T>({
  label,
  values,
  value,
  describe,
  onChoose,
  hint,
  preview,
}: {
  label: string;
  /** The values, in order along the slider. */
  values: readonly [T, ...T[]];
  /** The value chosen, one of `values`; the first stands in for any other. */
  value: T;
  describe: (value: T) => string;
  onChoose: (value: T) => void;
  hint?: string;
  /** What the value under the thumb looks like, shown beneath it while it moves too. */
  preview?: (value: T) => ReactNode;
}) {
  const at = (index: number): T => values[index] ?? values[0];
  const [moving, setMoving] = useState<number | null>(null);
  const shown = moving ?? Math.max(0, values.indexOf(value));
  const input = useRef<HTMLInputElement>(null);
  const text = describe(at(shown));
  // React Aria words a slider's value with a number format alone, which would read out the
  // position; the input's value text is set to the description once React Aria has written its
  // own, since this component's effects run after those of the slider inside it.
  useLayoutEffect(() => {
    input.current?.setAttribute("aria-valuetext", text);
  });
  return (
    <Slider
      value={shown}
      minValue={0}
      maxValue={values.length - 1}
      step={1}
      onChange={(index) => {
        if (typeof index === "number") {
          setMoving(index);
        }
      }}
      onChangeEnd={(index) => {
        if (typeof index === "number") {
          setMoving(null);
          onChoose(at(index));
        }
      }}
      className="flex w-full flex-col gap-1"
    >
      <div className="flex items-center justify-between gap-2">
        <Label className="text-sm font-medium">{label}</Label>
        <SliderOutput className="text-sm tabular-nums text-ink-muted">{text}</SliderOutput>
      </div>
      <SliderRail inputRef={input} />
      {preview?.(at(shown))}
      {hint !== undefined && (
        <Text slot="description" className="text-xs text-ink-muted">
          {hint}
        </Text>
      )}
    </Slider>
  );
}
