import type { Category } from "@aspen/protocol";
import { useState } from "react";
import {
  Button,
  Dialog,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  TextField,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** Renames a category, for those who may manage categories. */
export function RenameCategoryDialog({
  category,
  isOpen,
  onOpenChange,
}: {
  category: Category;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [name, setName] = useState(category.name);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>
            {format(m.channelLayout.renameCategory, { category: category.name })}
          </DialogHeading>
          <Form
            onSubmit={(event) => {
              event.preventDefault();
              setSaving(true);
              setError(null);
              sync.renameCategory(category.id, name.trim()).then(
                () => {
                  setSaving(false);
                  onOpenChange(false);
                },
                (e: unknown) => {
                  setSaving(false);
                  setError(problemText(e));
                },
              );
            }}
            className="flex flex-col gap-3"
          >
            <TextField value={name} onChange={setName} isRequired autoFocus className={fieldClass}>
              <Label className={labelClass}>{m.categoryNameLabel}</Label>
              <Input className={inputClass} />
            </TextField>
            {error !== null && (
              <p role="alert" className={alertClass}>
                {error}
              </p>
            )}
            <Button
              type="submit"
              isDisabled={saving || name.trim() === "" || name.trim() === category.name}
              className={primaryButtonClass + " self-start"}
            >
              {m.channelActions.save}
            </Button>
          </Form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** Deletes a category after saying what becomes of its channels. */
export function DeleteCategoryDialog({
  category,
  isOpen,
  onOpenChange,
}: {
  category: Category;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role="alertdialog" className={dialogClass}>
          <DialogHeading>
            {format(m.channelLayout.deleteCategoryHeading, { category: category.name })}
          </DialogHeading>
          <p className={hintClass}>{m.channelLayout.deleteCategoryHint}</p>
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <Button
            isDisabled={deleting}
            onPress={() => {
              setDeleting(true);
              setError(null);
              sync.deleteCategory(category.id).then(
                () => {
                  onOpenChange(false);
                },
                (e: unknown) => {
                  setDeleting(false);
                  setError(problemText(e));
                },
              );
            }}
            className={dangerButtonClass + " self-start"}
          >
            {m.channelLayout.confirmDeleteCategory}
          </Button>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
