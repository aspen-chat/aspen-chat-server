import aspenIcon from "../../../../../brand/aspen-icon.svg";

/** The side of the icon on the signed-out screens, in CSS pixels, where there is room for it. */
export const ICON_PX = 256;

/**
 * The size of the icon on the signed-out screens: half of `ICON_PX` on a phone-sized screen,
 * where the full size would push the forms' buttons below the fold, and `ICON_PX` from
 * Tailwind's `md` up, the width at which the app shows more than one pane.
 */
export const iconSizeClass = "size-32 md:size-64";

/**
 * Aspen's icon (`brand/aspen-icon.svg`, small enough that the build inlines it), as the
 * signed-out screens show it: above the welcome to choose a deployment, and in place of a
 * deployment's own icon when it has none. It is a tile with its own rounded corners, so it is
 * shown as drawn. Decorative: the welcome beside it says the same.
 */
export function AspenIcon() {
  return (
    <img
      src={aspenIcon}
      alt=""
      width={ICON_PX}
      height={ICON_PX}
      className={`${iconSizeClass} select-none`}
    />
  );
}
