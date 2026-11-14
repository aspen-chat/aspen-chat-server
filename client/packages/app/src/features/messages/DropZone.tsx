import { useEffect, useRef, useState, type DragEvent, type ReactNode } from "react";
import { DropZoneContext, type DropReceiver } from "@/features/messages/dropZoneContext";
import { useMessages } from "@/i18n/context";

/**
 * A pane that takes what is dropped on it for the message box inside it (`useDropReceiver`):
 * files are attached and text written in, as a paste would be. While files are dragged over it
 * a veil says they will be attached. Text dragged into the box itself is left to the box, which
 * puts it where it is dropped and moves it when it came from the box. A pane whose box is not
 * offered (the reader may not post there) takes nothing, and the drop is the browser's.
 */
export function DropZone({ className, children }: { className: string; children: ReactNode }) {
  const m = useMessages();
  const receiver = useRef<DropReceiver | null>(null);
  const [offered, setOffered] = useState(false);
  // Entering a child of the pane leaves the one before it, so the pane counts what it is in.
  const depth = useRef(0);
  const [veiled, setVeiled] = useState(false);
  const [register] = useState(() => (next: DropReceiver | null) => {
    receiver.current = next;
    setOffered(next !== null);
  });

  // A drag that ends outside the pane, or is dropped elsewhere, never leaves it by the count.
  useEffect(() => {
    if (!veiled) {
      return;
    }
    const lift = () => {
      depth.current = 0;
      setVeiled(false);
    };
    window.addEventListener("dragend", lift);
    window.addEventListener("drop", lift);
    return () => {
      window.removeEventListener("dragend", lift);
      window.removeEventListener("drop", lift);
    };
  }, [veiled]);

  const takes = (event: DragEvent) =>
    offered &&
    event.dataTransfer.types.some(
      (type) => type === "Files" || type === "text/plain" || type === "text/uri-list",
    );
  const carriesFiles = (event: DragEvent) => event.dataTransfer.types.includes("Files");

  return (
    <div
      className={`relative ${className}`}
      onDragEnter={(event) => {
        if (!takes(event)) {
          return;
        }
        event.preventDefault();
        depth.current += 1;
        if (carriesFiles(event)) {
          setVeiled(true);
        }
      }}
      onDragOver={(event) => {
        if (!takes(event)) {
          return;
        }
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
      }}
      onDragLeave={() => {
        depth.current = Math.max(0, depth.current - 1);
        if (depth.current === 0) {
          setVeiled(false);
        }
      }}
      onDrop={(event) => {
        depth.current = 0;
        setVeiled(false);
        const take = receiver.current;
        if (take === null || !takes(event)) {
          return;
        }
        if (!carriesFiles(event) && event.target instanceof HTMLTextAreaElement) {
          return;
        }
        event.preventDefault();
        take(event.dataTransfer);
      }}
    >
      <DropZoneContext.Provider value={register}>{children}</DropZoneContext.Provider>
      {veiled && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-2 z-20 flex items-center justify-center rounded-lg border-2 border-dashed border-accent bg-surface/85 text-lg font-semibold text-accent"
        >
          {m.dropToAttach}
        </div>
      )}
    </div>
  );
}
