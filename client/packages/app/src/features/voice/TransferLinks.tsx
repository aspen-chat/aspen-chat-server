import type { TransferLink } from "@aspen/protocol";
import { type RefObject, useLayoutEffect, useState } from "react";
import { useUser } from "@/api/hooks";
import { displayNameOf } from "@/features/users/profile";
import { type Box, linkPath } from "@/features/voice/files";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The gap between call tiles, in pixels (`gap-3`). */
const TILE_GAP = 12;

/**
 * The transfers under way in the call, drawn over its tiles as dashed lines in the accent colour
 * that march from sender to receiver, turning only at right angles. It shows that files are
 * moving and between whom, never what or how fast. A reader who asks for less motion sees the
 * lines still, and a screen reader hears them as a list.
 */
export function TransferLinks({
  links,
  container,
}: {
  links: readonly TransferLink[];
  container: RefObject<HTMLElement | null>;
}) {
  const [boxes, setBoxes] = useState<ReadonlyMap<string, Box>>(new Map());
  useLayoutEffect(() => {
    const element = container.current;
    if (element === null || links.length === 0) {
      return;
    }
    const measure = () => {
      const origin = element.getBoundingClientRect();
      const measured = new Map<string, Box>();
      for (const tile of element.querySelectorAll<HTMLElement>("[data-voice-tile]")) {
        const rect = tile.getBoundingClientRect();
        const id = tile.dataset.voiceTile;
        if (id !== undefined) {
          measured.set(id, {
            left: rect.left - origin.left,
            top: rect.top - origin.top,
            width: rect.width,
            height: rect.height,
          });
        }
      }
      setBoxes(measured);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [container, links]);

  if (links.length === 0) {
    return null;
  }
  // Several links between the same two tiles each take a lane of their own.
  const lanes = new Map<string, number>();
  const paths = links.flatMap((link, index) => {
    const from = boxes.get(link.sender);
    const to = boxes.get(link.receiver);
    if (from === undefined || to === undefined) {
      return [];
    }
    const pair = [link.sender, link.receiver].sort().join(":");
    const lane = lanes.get(pair) ?? 0;
    lanes.set(pair, lane + 1);
    return [{ key: `${pair}:${String(index)}`, d: linkPath(from, to, TILE_GAP, lane) }];
  });
  return (
    <>
      <svg
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 h-full w-full overflow-visible"
      >
        {paths.map((path) => (
          <path
            key={path.key}
            d={path.d}
            fill="none"
            strokeWidth={2}
            strokeDasharray="6 6"
            strokeLinejoin="round"
            className="animate-march stroke-accent motion-reduce:animate-none"
          />
        ))}
      </svg>
      <LinkList links={links} />
    </>
  );
}

function LinkList({ links }: { links: readonly TransferLink[] }) {
  const m = useMessages();
  return (
    <ul aria-label={m.files.linksLabel} className="sr-only">
      {links.map((link, index) => (
        <LinkItem key={`${link.sender}:${link.receiver}:${String(index)}`} link={link} />
      ))}
    </ul>
  );
}

function LinkItem({ link }: { link: TransferLink }) {
  const m = useMessages();
  const sender = useUser(link.sender);
  const receiver = useUser(link.receiver);
  return (
    <li>
      {format(m.files.link, {
        sender: sender === undefined ? m.unknownUser : displayNameOf(sender),
        receiver: receiver === undefined ? m.unknownUser : displayNameOf(receiver),
      })}
    </li>
  );
}
