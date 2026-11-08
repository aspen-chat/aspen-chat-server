// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Button, Dialog, Modal, ModalOverlay } from "react-aria-components";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UnsavedChangesGuard } from "./UnsavedChangesGuard";
import { useUnsavedChanges } from "./unsavedChanges";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** A form that reports `dirty`, with a button that saves and then closes through `close`. */
function Form({ dirty, close }: { dirty: boolean; close: () => void }) {
  const saved = useUnsavedChanges(dirty);
  return (
    <Button
      onPress={() => {
        saved();
        close();
      }}
    >
      Save
    </Button>
  );
}

function Guarded({
  dirty,
  formDirty,
  onOpenChange,
}: {
  dirty?: boolean;
  formDirty: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <UnsavedChangesGuard
      isOpen
      onOpenChange={onOpenChange}
      {...(dirty === undefined ? {} : { dirty })}
    >
      {(guard) => (
        <ModalOverlay isOpen={guard.isOpen} onOpenChange={guard.onOpenChange} isDismissable>
          <Modal>
            <Dialog aria-label="Edit">
              {({ close }) => <Form dirty={formDirty} close={close} />}
            </Dialog>
          </Modal>
        </ModalOverlay>
      )}
    </UnsavedChangesGuard>
  );
}

const question = () => document.querySelector('[role="alertdialog"]');
/** A button by its text, or by its name alone when it shows an icon (the question's X). */
const button = (name: string) =>
  [...document.querySelectorAll("button")].find(
    (b) => b.textContent === name || b.getAttribute("aria-label") === name,
  );
const press = async (element: Element | undefined) => {
  if (element === undefined) {
    throw new Error("no such button");
  }
  // Async, so what a press starts (the question's discard, which closes once its promise
  // settles) has settled before the next assertion.
  await act(async () => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await Promise.resolve();
  });
};

/** Escape pressed in the guarded dialog, as a reader with focus in it would. */
const escape = () => {
  act(() => {
    document
      .querySelector('[role="dialog"][aria-label="Edit"]')
      ?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  });
};

describe("UnsavedChangesGuard", () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(() => {
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(() => {
    act(() => {
      root.unmount();
    });
    container.remove();
  });
  const render = (element: React.ReactNode) => {
    act(() => {
      root.render(element);
    });
  };

  it("closes at once while nothing inside has changed", () => {
    const onOpenChange = vi.fn();
    render(<Guarded formDirty={false} onOpenChange={onOpenChange} />);
    escape();
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(question()).toBeNull();
  });

  it("asks before closing on a form's unsaved changes, and closes only on discard", async () => {
    const onOpenChange = vi.fn();
    render(<Guarded formDirty onOpenChange={onOpenChange} />);
    escape();
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(question()?.textContent).toContain("Discard your changes?");
    // The question dismissed keeps editing.
    await press(button("Close"));
    expect(question()).toBeNull();
    expect(onOpenChange).not.toHaveBeenCalled();
    escape();
    await press(button("Discard changes"));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(question()).toBeNull();
  });

  it("asks on the modal's own dirty flag too", () => {
    const onOpenChange = vi.fn();
    render(<Guarded dirty formDirty={false} onOpenChange={onOpenChange} />);
    escape();
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(question()).not.toBeNull();
  });

  it("lets a form that has just saved close without asking", async () => {
    const onOpenChange = vi.fn();
    render(<Guarded formDirty onOpenChange={onOpenChange} />);
    await press(button("Save"));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(question()).toBeNull();
  });
});
