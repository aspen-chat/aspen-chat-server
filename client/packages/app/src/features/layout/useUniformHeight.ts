import { useLayoutEffect, useRef } from "react";

/**
 * Sets `--choice-height` on the element it is given to the natural height of the tallest
 * `label` within it, for choices that take it as their least height (`UNIFORM_CHOICE_CLASS`). It measures again
 * whenever the element's size changes (the width, which rewraps every hint, or the text), each
 * time with the variable cleared, so the choices' own heights are what is compared.
 */
export function useUniformHeight() {
  const list = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = list.current;
    if (element === null) {
      return;
    }
    const fit = () => {
      element.style.removeProperty("--choice-height");
      const heights = Array.from(element.querySelectorAll("label"), (label) => label.offsetHeight);
      element.style.setProperty("--choice-height", `${String(Math.max(0, ...heights))}px`);
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, []);
  return list;
}
