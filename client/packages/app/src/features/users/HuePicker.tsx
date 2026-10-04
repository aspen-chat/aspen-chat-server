import {
  ColorSlider,
  ColorThumb,
  Label,
  SliderOutput,
  SliderTrack,
  parseColor,
} from "react-aria-components";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";
import { hueColor } from "@/theme/nameColors";

/** The hue a role is offered when it is first given a colour: a blue. */
const FIRST_HUE = 210;

/**
 * A role's colour: whether it has one, and if so its hue around the wheel, with the name it
 * colours shown as it will be drawn on a light ground and a dark one. Only the hue is chosen;
 * how light and how strong it is drawn is the app's, so that it reads everywhere.
 */
export function HuePicker({
  hue,
  onChange,
  sample,
  hint,
  isDisabled = false,
}: {
  hue: number | null;
  onChange: (hue: number | null) => void;
  /** The name the preview draws. */
  sample: string;
  /** Where the colour shows, and over what. */
  hint: string;
  isDisabled?: boolean;
}) {
  const m = useMessages();
  return (
    <div className="flex flex-col gap-2">
      <ChoiceCheckbox
        isSelected={hue !== null}
        onChange={(selected) => {
          onChange(selected ? FIRST_HUE : null);
        }}
        isDisabled={isDisabled}
        label={m.roles.colorLabel}
        hint={hint}
      />
      {hue !== null && (
        <>
          <ColorSlider
            channel="hue"
            colorSpace="hsb"
            value={parseColor(`hsb(${String(hue)}, 100%, 100%)`)}
            onChange={(color) => {
              onChange(Math.round(color.getChannelValue("hue")) % 360);
            }}
            isDisabled={isDisabled}
            className="flex flex-col gap-1"
          >
            <div className="flex items-baseline justify-between text-sm">
              <Label className="font-medium">{m.roles.hueLabel}</Label>
              <SliderOutput className="text-ink-muted tabular-nums" />
            </div>
            <SliderTrack
              className="h-6 rounded-md"
              // The track is drawn in the colours it chooses between.
              style={({ defaultStyle }) => ({ ...defaultStyle, borderRadius: undefined })}
            >
              <ColorThumb className="top-1/2 h-6 w-6 rounded-full border-2 border-surface-raised shadow ring-1 ring-ink/40 focus-visible:ring-2 focus-visible:ring-accent" />
            </SliderTrack>
          </ColorSlider>
          <div aria-hidden="true" className="flex flex-wrap gap-2 text-sm font-medium">
            {(["light", "dark"] as const).map((scheme) => (
              <span
                key={scheme}
                // Each preview takes one scheme, so the palette's tokens and the name's colour
                // both resolve as they would there.
                style={{ colorScheme: scheme }}
                className="rounded-md border border-line bg-surface-raised px-3 py-1"
              >
                <span style={{ color: hueColor(hue) }}>{sample}</span>
              </span>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
