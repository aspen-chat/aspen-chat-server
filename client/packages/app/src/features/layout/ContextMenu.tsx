import { useEffect, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { Dialog, Popover, type PopoverProps } from "react-aria-components";
import { UNDERLAY, type ContextMenuState } from "@/features/layout/contextMenuState";
import { menuPopoverClass } from "@/features/layout/menu";

/** What every React Aria popover is marked with, as it is placed. */
const POPOVER = "[data-placement]";

/**
 * What is at the point beneath every popover and the layer laid behind them, as the page will
 * be once they close: the page is let out of being inert for the moment it takes to look, since
 * an inert element is never found under a point.
 */
function beneathPopovers(x: number, y: number): Element | undefined {
  const inert = Array.from(document.querySelectorAll<HTMLElement>("[inert]"));
  for (const element of inert) {
    element.inert = false;
  }
  const beneath = document
    .elementsFromPoint(x, y)
    .find(
      (element) =>
        element.closest(`[data-testid=${UNDERLAY}]`) === null && element.closest(POPOVER) === null,
    );
  for (const element of inert) {
    element.inert = true;
  }
  return beneath;
}

/**
 * Passes a right click that lands on the page behind a popover (any popover: a menu, a profile
 * card) on to what is under it. While a popover is open, and while it closes, React Aria makes
 * the rest of the page inert and lays a layer over it, so the click lands on that layer or the
 * page's body, whose menu would be the browser's; and the popover may close as the button goes
 * down, before the click arrives. Every context menu still open closes first, since React Aria
 * closes only the topmost popover on a click outside it, which the one opened next would be.
 * With Shift held it is left to the browser.
 */
function passOn(event: MouseEvent) {
  const target = event.target;
  const behind =
    target instanceof HTMLElement &&
    (target.dataset.testid === UNDERLAY ||
      ((target === document.body || target === document.documentElement) &&
        document.querySelector("[inert]") !== null));
  if (!behind || event.shiftKey) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  for (const close of openMenus) {
    close();
  }
  beneathPopovers(event.clientX, event.clientY)?.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: event.clientX,
      clientY: event.clientY,
      button: 2,
    }),
  );
}

/** How to close each context menu open now. */
const openMenus = new Set<() => void>();

if (typeof document !== "undefined") {
  document.addEventListener("contextmenu", passOn, true);
}

/**
 * The element a context menu is placed against: nothing at its point, fixed to the viewport.
 * It goes at the end of the page, since a fixed element inside one that moves or scrolls is
 * placed by that one instead. While its menu is open, a right click passed on to something
 * else closes it (`passOn`).
 */
export function MenuPoint({
  menu: { point, anchorRef, isOpen, onOpenChange },
}: {
  menu: ContextMenuState;
}) {
  useEffect(() => {
    if (!isOpen) {
      return;
    }
    const close = () => {
      onOpenChange(false);
    };
    openMenus.add(close);
    return () => {
      openMenus.delete(close);
    };
  }, [isOpen, onOpenChange]);
  if (point === null) {
    return null;
  }
  return createPortal(
    <span
      ref={anchorRef}
      aria-hidden="true"
      className="pointer-events-none fixed h-0 w-0"
      style={{ left: point.x, top: point.y }}
    />,
    document.body,
  );
}

/**
 * A context menu opened by `menu`, named `label`: at the pointer, its corner at the point and
 * turned to whichever side there is room on; or, opened by its button, beside `button` as
 * `placement` says. `children` are what it holds: a React Aria `Menu`, and anything above it. It
 * is built from when it first opens, and not before, since each of the hundreds of rows of a
 * list may have one.
 */
export function ContextMenu({
  menu,
  label,
  button,
  placement = "bottom end",
  children,
}: {
  menu: ContextMenuState;
  label: string;
  /** What opens it besides a right click, which it opens beside when that was what opened it. */
  button?: RefObject<HTMLElement | null>;
  placement?: PopoverProps["placement"];
  children: ReactNode;
}) {
  if (!menu.opened) {
    return null;
  }
  const atPoint = menu.from === "point" || button === undefined;
  return (
    <>
      <MenuPoint menu={menu} />
      <Popover
        triggerRef={atPoint ? menu.anchorRef : button}
        isOpen={menu.isOpen}
        onOpenChange={menu.onOpenChange}
        placement={atPoint ? "bottom start" : placement}
        offset={atPoint ? 2 : 8}
        className={menuPopoverClass}
      >
        <Dialog aria-label={label} className="flex flex-col outline-none">
          {/* A right click on the menu is the menu's: it is drawn inside what it is for, in the
              React tree, which would otherwise open its menu again where the click was. */}
          <div
            className="contents"
            onContextMenu={(event) => {
              event.preventDefault();
              event.stopPropagation();
            }}
          >
            {children}
          </div>
        </Dialog>
      </Popover>
    </>
  );
}
