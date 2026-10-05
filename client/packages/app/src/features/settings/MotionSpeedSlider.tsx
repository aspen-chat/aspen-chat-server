import { MAX_MOTION_SPEED, MOTION_SPEED } from "@aspen/protocol";
import { useState } from "react";
import { Label, Slider, SliderOutput, Text } from "react-aria-components";
import { usePreference, useSync } from "@/api/hooks";
import { SliderRail } from "@/features/layout/SliderRail";
import { useLanguageSetting, useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** How finely the speed is set. */
const STEP = 0.25;

/**
 * How fast animations run, from off (the far left) to twice the normal speed, kept with the
 * account so every device moves alike. It is written when the thumb is let go, not as it moves.
 */
export function MotionSpeedSlider() {
  const m = useMessages();
  const sync = useSync();
  const { resolved } = useLanguageSetting();
  const kept = usePreference(MOTION_SPEED);
  const [moving, setMoving] = useState<number | null>(null);
  const speed = moving ?? kept;
  const describe = (value: number) =>
    value === 0
      ? m.settings.motionOff
      : value === 1
        ? m.settings.motionNormal
        : format(m.settings.motionTimes, {
            speed: new Intl.NumberFormat(resolved.locale, { maximumFractionDigits: 2 }).format(
              value,
            ),
          });
  return (
    <Slider
      value={speed}
      minValue={0}
      maxValue={MAX_MOTION_SPEED}
      step={STEP}
      // Read out as a share of the normal speed: 100%, and 0% for off.
      formatOptions={{ style: "percent" }}
      onChange={(value) => {
        if (typeof value === "number") {
          setMoving(value);
        }
      }}
      onChangeEnd={(value) => {
        if (typeof value === "number") {
          setMoving(null);
          void sync.preferences.set(MOTION_SPEED, value);
        }
      }}
      className="flex w-full flex-col gap-1"
    >
      <div className="flex items-center justify-between gap-2">
        <Label className="text-sm font-medium">{m.settings.motionSpeed}</Label>
        <SliderOutput className="text-sm tabular-nums text-ink-muted">
          {({ state }) => describe(state.getThumbValue(0))}
        </SliderOutput>
      </div>
      <SliderRail />
      <Text slot="description" className="text-xs text-ink-muted">
        {m.settings.motionHint}
      </Text>
    </Slider>
  );
}
