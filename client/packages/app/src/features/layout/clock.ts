/**
 * A number of seconds, cut down to whole ones, as `m:ss` under an hour and `h:mm:ss` from there.
 * With `longest`, it takes as many places as `longest` would, so a position and the length it is
 * shown against line up (`0:05:03` of `1:02:00`).
 */
export function formatClock(seconds: number, longest = seconds): string {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secondsPart = String(total % 60).padStart(2, "0");
  return hours > 0 || longest >= 3600
    ? `${String(hours)}:${String(minutes).padStart(2, "0")}:${secondsPart}`
    : `${String(minutes)}:${secondsPart}`;
}
