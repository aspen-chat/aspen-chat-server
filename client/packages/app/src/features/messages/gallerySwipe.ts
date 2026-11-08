/** How fast a finger must be moving as it lifts to throw the gallery to the next picture, in px/ms. */
export const FLICK = 0.3;

/**
 * Which picture a swipe lands on when the finger lifts: `1` the next, `-1` the previous, `0`
 * the one it was on. `travel` is how far the finger moved along the inline axis since it
 * touched, positive toward the inline end, and `velocity` how fast it was moving as it lifted,
 * the same way; `width` is the stage's, which a picture fills. A finger moving toward the end
 * drags the previous picture in from the start, so a throw or a drag past halfway that way
 * lands on the previous picture, and the other way on the next; a finger that came to rest
 * short of halfway, or one thrown toward a picture that is not there, lands where it was.
 */
export function swipeOutcome(
  travel: number,
  velocity: number,
  width: number,
  { hasPrevious, hasNext }: { hasPrevious: boolean; hasNext: boolean },
): -1 | 0 | 1 {
  const toward: -1 | 0 | 1 =
    velocity < -FLICK
      ? 1
      : velocity > FLICK
        ? -1
        : Math.abs(travel) > width / 2
          ? travel < 0
            ? 1
            : -1
          : 0;
  if (toward === 1 && !hasNext) {
    return 0;
  }
  if (toward === -1 && !hasPrevious) {
    return 0;
  }
  return toward;
}
