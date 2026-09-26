import { useEffect, useRef } from "react";

/**
 * One shared screen, playing. The audio that came with it is played by the call itself, so the
 * element is muted and only shows the picture. A local preview is the sender's own track.
 */
export function ScreenTile({
  track,
  label,
  className,
}: {
  track: MediaStreamTrack;
  label: string;
  className?: string;
}) {
  const video = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const element = video.current;
    if (element === null) {
      return;
    }
    element.srcObject = new MediaStream([track]);
    return () => {
      element.srcObject = null;
    };
  }, [track]);
  return (
    <figure className={"relative overflow-hidden rounded-lg bg-black " + (className ?? "")}>
      <video
        ref={video}
        autoPlay
        playsInline
        muted
        aria-label={label}
        className="h-full w-full object-contain"
      />
      <figcaption className="absolute bottom-2 left-2 rounded-md bg-black/60 px-2 py-0.5 text-xs text-white">
        {label}
      </figcaption>
    </figure>
  );
}
