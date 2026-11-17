import { EMOTICON_EMOJI } from "@aspen/protocol";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type Dispatch,
  type RefObject,
  type SetStateAction,
} from "react";
import { usePreference } from "@/api/hooks";
import {
  convertLast,
  convertTyped,
  emoticonsIn,
  type Conversion,
} from "@/features/emoji/emoticons";
import { retype, type PickedText, type Span } from "@/features/messages/picks";

/**
 * Keys that leave a conversion to be taken back: Backspace itself, modifiers, and a virtual
 * keyboard's keys, which say what they did only in the input they make.
 */
const KEEPS_CONVERSION = new Set([
  "Backspace",
  "Shift",
  "Control",
  "Alt",
  "Meta",
  "Unidentified",
  "Process",
]);

/** Spans of a text, with the text they were found in. */
interface Declined {
  readonly text: string;
  readonly spans: readonly Span[];
}

/** `declined`'s spans carried to `text`, as an edit made elsewhere (a completion) left it. */
function spansIn(declined: Declined, text: string): readonly Span[] {
  return declined.text === text
    ? declined.spans
    : retype({ text: declined.text, picks: declined.spans }, text).picks;
}

/**
 * Turns emoticons typed in a message box into emoji (`emoticons.ts`), while `EMOTICON_EMOJI`
 * is on. An emoticon becomes its emoji as the space, line's end, or punctuation after it is
 * typed, and one that ends the message as it is sent (`outgoing`). Backspace pressed next puts
 * the emoticon back as it was typed, deleting nothing, and that one stays as typed for as long
 * as it stands in the box: typing or moving the caret first gives that up. The emoticons the
 * box opens with (`kept`, a sent message's being edited) stay as they were written.
 *
 * `type` takes the box's own edits in place of `usePickedText`'s. Backspace is heard on `box`
 * before the box deletes anything, through `beforeinput`, which every keyboard reports it by.
 */
export function useEmoticons({
  written,
  box,
  off = false,
  kept = "",
}: {
  written: {
    held: PickedText;
    type: (next: string, caret?: number) => void;
    set: Dispatch<SetStateAction<PickedText>>;
  };
  box: RefObject<HTMLTextAreaElement | null>;
  /** Converts nothing, while something else reads what is typed (a command line). */
  off?: boolean;
  kept?: string;
}) {
  const on = usePreference(EMOTICON_EMOJI) && !off;
  const { held, type: retyped, set } = written;
  // The emoticons Backspace put back, which stay as typed.
  const [declined, setDeclined] = useState<Declined>(() => ({
    text: kept,
    spans: emoticonsIn(kept),
  }));
  const undo = useRef<Conversion | null>(null);
  const placing = useRef<number | null>(null);
  const composing = useRef(false);

  const type = (next: string, caret?: number) => {
    undo.current = null;
    const was = held.text;
    const spans = retype({ text: was, picks: spansIn(declined, was) }, next, caret).picks;
    const conversion =
      on && !composing.current && caret !== undefined
        ? convertTyped(retype(held, next, caret), was, caret, spans)
        : null;
    if (conversion === null) {
      setDeclined({ text: next, spans });
      retyped(next, caret);
      return;
    }
    setDeclined({
      text: conversion.held.text,
      spans: retype({ text: next, picks: spans }, conversion.held.text, conversion.caret).picks,
    });
    set(conversion.held);
    undo.current = conversion;
    placing.current = conversion.caret;
  };

  /** The box's text as it is sent: an emoticon left at its end becomes its emoji. */
  const outgoing = (text: PickedText): PickedText =>
    on ? convertLast(text, spansIn(declined, text.text)) : text;

  // The caret goes where the conversion or its undoing leaves it as the text is committed,
  // before anything else typed can land.
  useLayoutEffect(() => {
    const at = placing.current;
    if (at !== null) {
      placing.current = null;
      box.current?.setSelectionRange(at, at);
    }
  });

  // Heard on the document for whichever element the box is now: a box drawn only once its
  // reader may post comes and goes with that.
  useEffect(() => listen(box, { undo, placing, composing, set, setDeclined }), [box, set]);

  return { type, outgoing };
}

/**
 * Listens for Backspace in `box` just after a conversion, to take it back, and for whatever
 * gives that up; returns what stops listening.
 */
function listen(
  box: RefObject<HTMLTextAreaElement | null>,
  state: {
    undo: RefObject<Conversion | null>;
    placing: RefObject<number | null>;
    composing: RefObject<boolean>;
    set: Dispatch<SetStateAction<PickedText>>;
    setDeclined: Dispatch<SetStateAction<Declined>>;
  },
): () => void {
  const { undo, placing, composing, set, setDeclined } = state;
  const inBox = (event: Event) => box.current !== null && event.target === box.current;
  const forget = (event: Event) => {
    if (inBox(event)) {
      undo.current = null;
    }
  };
  const onBeforeInput = (event: InputEvent) => {
    const element = box.current;
    if (element === null || event.target !== element) {
      return;
    }
    const conversion = undo.current;
    undo.current = null;
    if (
      conversion === null ||
      event.inputType !== "deleteContentBackward" ||
      element.value !== conversion.held.text ||
      element.selectionStart !== conversion.caret ||
      element.selectionEnd !== conversion.caret
    ) {
      return;
    }
    event.preventDefault();
    const typed = conversion.typed.text;
    setDeclined((declined) => ({
      text: typed,
      spans: [
        ...retype(
          { text: conversion.held.text, picks: spansIn(declined, conversion.held.text) },
          typed,
          conversion.typedCaret,
        ).picks,
        conversion.emoticon,
      ].sort((a, b) => a.start - b.start),
    }));
    set(conversion.typed);
    placing.current = conversion.typedCaret;
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (!KEEPS_CONVERSION.has(event.key)) {
      forget(event);
    }
  };
  const onCompositionStart = (event: Event) => {
    if (inBox(event)) {
      composing.current = true;
    }
  };
  const onCompositionEnd = (event: Event) => {
    if (inBox(event)) {
      composing.current = false;
    }
  };
  // Captured, so a handler of the box's own that stops an event does not hide it here.
  const options = { capture: true };
  document.addEventListener("beforeinput", onBeforeInput, options);
  document.addEventListener("keydown", onKeyDown, options);
  document.addEventListener("pointerdown", forget, options);
  document.addEventListener("focusout", forget, options);
  document.addEventListener("compositionstart", onCompositionStart, options);
  document.addEventListener("compositionend", onCompositionEnd, options);
  return () => {
    document.removeEventListener("beforeinput", onBeforeInput, options);
    document.removeEventListener("keydown", onKeyDown, options);
    document.removeEventListener("pointerdown", forget, options);
    document.removeEventListener("focusout", forget, options);
    document.removeEventListener("compositionstart", onCompositionStart, options);
    document.removeEventListener("compositionend", onCompositionEnd, options);
    composing.current = false;
  };
}
