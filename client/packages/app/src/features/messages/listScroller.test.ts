import type { TouchEvent as ReactTouchEvent } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ListScroller } from "./listScroller";

/** A list taller than its box, with a link in it for a finger to land on. */
function makeList(): {
  scroller: ListScroller;
  box: HTMLDivElement;
  content: HTMLDivElement;
  track: HTMLDivElement;
  link: HTMLAnchorElement;
  sizes: { scrollHeight: number; clientHeight: number };
} {
  const box = document.createElement("div");
  const content = document.createElement("div");
  const track = document.createElement("div");
  const bar = document.createElement("div");
  const link = document.createElement("a");
  content.append(link);
  track.append(bar);
  box.append(content, track);
  document.body.append(box);
  // jsdom lays nothing out: the box's sizes and position are its own numbers.
  const sizes = { scrollHeight: 2000, clientHeight: 500 };
  let scrollTop = 0;
  Object.defineProperties(box, {
    scrollHeight: { get: () => sizes.scrollHeight },
    clientHeight: { get: () => sizes.clientHeight },
    scrollTop: {
      get: () => scrollTop,
      set: (value: number) => {
        scrollTop = Math.max(0, Math.min(sizes.scrollHeight - sizes.clientHeight, value));
      },
    },
  });
  const scroller = new ListScroller({
    viewport: { current: box },
    content: { current: content },
    indicator: { current: track },
    thumb: { current: bar },
  });
  scroller.update({
    atLatest: true,
    highlightId: undefined,
    events: { moved: () => undefined, steered: () => undefined, farBack: () => undefined },
  });
  scroller.measureRange();
  return { scroller, box, content, track, link, sizes };
}

/** A finger's event as React hands it to the list: a touch at `y` on `target`. */
function touch(target: EventTarget, y: number): ReactTouchEvent<HTMLDivElement> {
  return { target, touches: [{ clientY: y }] } as unknown as ReactTouchEvent<HTMLDivElement>;
}

/** The finger's lift, and whether the list kept the browser from following it with a click. */
function lift(target: EventTarget): {
  event: ReactTouchEvent<HTMLDivElement>;
  prevented: () => boolean;
} {
  let prevented = false;
  const event = {
    target,
    touches: [],
    preventDefault: () => {
      prevented = true;
    },
  } as unknown as ReactTouchEvent<HTMLDivElement>;
  return { event, prevented: () => prevented };
}

describe("ListScroller's fingers", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    document.body.replaceChildren();
  });

  it("moves nothing for a tap, and follows a drag from where it passed the slop", () => {
    const { scroller, link, content } = makeList();
    scroller.onTouchStart(touch(link, 300));
    scroller.onTouchMove(touch(link, 295));
    expect(scroller.current()).toBe(0);
    expect(content.style.pointerEvents).toBe("");

    scroller.onTouchMove(touch(link, 290));
    // Ten up the screen, of which the slop's eight is not a move.
    expect(scroller.current()).toBe(2);
    expect(content.style.pointerEvents).toBe("none");

    scroller.onTouchMove(touch(link, 280));
    expect(scroller.current()).toBe(12);
  });

  it("takes a touch on a resting list as a tap", () => {
    const { scroller, link, content } = makeList();
    const cancelled = vi.fn();
    link.addEventListener("pointercancel", cancelled);
    scroller.onTouchStart(touch(link, 300));
    const lifted = lift(link);
    scroller.onTouchEnd(lifted.event);

    expect(cancelled).not.toHaveBeenCalled();
    expect(lifted.prevented()).toBe(false);
    expect(content.style.pointerEvents).toBe("");
  });

  it("ends the press of what a drag began on, and makes its lift no click", () => {
    const { scroller, link, content } = makeList();
    const cancelled = vi.fn();
    link.addEventListener("pointercancel", cancelled);
    scroller.onTouchStart(touch(link, 300));
    scroller.onTouchMove(touch(link, 280));
    expect(cancelled).toHaveBeenCalledTimes(1);
    expect((cancelled.mock.calls[0]?.[0] as PointerEvent).pointerType).toBe("touch");

    scroller.onTouchMove(touch(link, 270));
    expect(cancelled).toHaveBeenCalledTimes(1);
    const lifted = lift(link);
    scroller.onTouchEnd(lifted.event);
    expect(lifted.prevented()).toBe(true);
    expect(content.style.pointerEvents).toBe("");
  });

  it("takes a touch that stops a coasting list as the stop alone, never a tap", () => {
    const { scroller, link, content } = makeList();
    scroller.onTouchStart(touch(link, 400));
    for (let y = 390; y >= 300; y -= 10) {
      vi.advanceTimersByTime(8);
      scroller.onTouchMove(touch(link, y));
    }
    scroller.onTouchEnd(lift(link).event);
    const atLift = scroller.current();
    vi.advanceTimersByTime(48);
    expect(scroller.current()).toBeGreaterThan(atLift);

    const cancelled = vi.fn();
    link.addEventListener("pointercancel", cancelled);
    scroller.onTouchStart(touch(link, 350));
    const stopped = scroller.current();
    expect(cancelled).toHaveBeenCalledTimes(1);
    expect(content.style.pointerEvents).toBe("none");
    vi.advanceTimersByTime(48);
    expect(scroller.current()).toBe(stopped);

    const lifted = lift(link);
    scroller.onTouchEnd(lifted.event);
    expect(lifted.prevented()).toBe(true);
    expect(content.style.pointerEvents).toBe("");
    vi.advanceTimersByTime(48);
    expect(scroller.current()).toBe(stopped);
  });

  it("puts the keyboard away as a drag begins", () => {
    const { scroller, link } = makeList();
    const field = document.createElement("textarea");
    document.body.append(field);
    field.focus();
    scroller.onTouchStart(touch(link, 300));
    scroller.onTouchMove(touch(link, 296));
    expect(document.activeElement).toBe(field);

    scroller.onTouchMove(touch(link, 280));
    expect(document.activeElement).toBe(document.body);
  });
});

describe("ListScroller's indicator", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    document.body.replaceChildren();
  });

  it("flashes as the list appears, and is hidden while everything fits", () => {
    const { scroller, track, sizes } = makeList();
    scroller.update({
      atLatest: false,
      highlightId: undefined,
      events: { moved: () => undefined, steered: () => undefined, farBack: () => undefined },
    });
    scroller.reset();
    scroller.position();
    expect(track.dataset.moving).toBe("");
    expect(track.style.visibility).toBe("");
    vi.advanceTimersByTime(1000);
    expect(track.dataset.moving).toBeUndefined();

    scroller.position();
    expect(track.dataset.moving).toBeUndefined();

    sizes.scrollHeight = 400;
    scroller.update({
      atLatest: true,
      highlightId: undefined,
      events: { moved: () => undefined, steered: () => undefined, farBack: () => undefined },
    });
    scroller.reset();
    scroller.position();
    expect(track.style.visibility).toBe("hidden");
  });
});
