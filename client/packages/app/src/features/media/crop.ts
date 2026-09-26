/** A circle inside an image, in the image's own pixels. */
export interface Circle {
  /** The centre, from the image's top-left corner. */
  x: number;
  y: number;
  radius: number;
}

/** The smallest circle worth cropping to, in image pixels. */
export const MIN_RADIUS = 16;
/** The longest side of a stored icon; a larger crop is scaled down to this. */
export const ICON_MAX_SIZE = 512;

/** The largest radius a circle inside a `width` by `height` image can have. */
export function maxRadius(width: number, height: number): number {
  return Math.max(MIN_RADIUS, Math.min(width, height) / 2);
}

/** The starting selection: the largest centred circle. */
export function initialCircle(width: number, height: number): Circle {
  return { x: width / 2, y: height / 2, radius: maxRadius(width, height) };
}

/**
 * The nearest circle to `circle` that lies wholly inside the image. The radius is first held to
 * what fits, then the centre is kept at least a radius from every edge, so the circle can sit
 * against an edge or in a corner but never cross one.
 */
export function clampCircle(circle: Circle, width: number, height: number): Circle {
  const radius = Math.min(Math.max(circle.radius, MIN_RADIUS), maxRadius(width, height));
  return {
    x: Math.min(Math.max(circle.x, radius), width - radius),
    y: Math.min(Math.max(circle.y, radius), height - radius),
    radius,
  };
}

/** The square the circle inscribes, as a source rectangle for drawing. */
export function boundingSquare(circle: Circle): { x: number; y: number; size: number } {
  return { x: circle.x - circle.radius, y: circle.y - circle.radius, size: circle.radius * 2 };
}

/** The side of the icon to make from a circle: its diameter, capped at `ICON_MAX_SIZE`. */
export function iconSize(circle: Circle): number {
  return Math.min(Math.round(circle.radius * 2), ICON_MAX_SIZE);
}

/** How large the image is shown at, so it fits in `maxWidth` by `maxHeight` without growing. */
export function fitScale(
  width: number,
  height: number,
  maxWidth: number,
  maxHeight: number,
): number {
  return Math.min(1, maxWidth / width, maxHeight / height);
}
