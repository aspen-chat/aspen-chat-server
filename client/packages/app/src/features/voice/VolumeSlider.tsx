import { MAX_USER_VOLUME } from "@aspen/protocol";
import { Label, Slider, SliderOutput } from "react-aria-components";
import { SliderRail } from "@/features/layout/SliderRail";

const STEP = 5;

/** A volume, 0 to `MAX_USER_VOLUME` as a percentage, in steps of `STEP`. */
export function VolumeSlider({
  label,
  gain,
  isDisabled,
  onChange,
}: {
  label: string;
  gain: number;
  isDisabled?: boolean;
  onChange: (gain: number) => void;
}) {
  return (
    <Slider
      value={Math.round(gain * 100)}
      minValue={0}
      maxValue={MAX_USER_VOLUME * 100}
      step={STEP}
      isDisabled={isDisabled ?? false}
      onChange={(value) => {
        if (typeof value === "number") {
          onChange(value / 100);
        }
      }}
      className="flex w-full flex-col gap-1 px-1 disabled:opacity-60"
    >
      <div className="flex items-center justify-between gap-2">
        <Label className="truncate text-sm font-medium">{label}</Label>
        <SliderOutput className="text-sm tabular-nums text-ink-muted">
          {({ state }) => `${String(state.getThumbValue(0))}%`}
        </SliderOutput>
      </div>
      <SliderRail />
    </Slider>
  );
}
