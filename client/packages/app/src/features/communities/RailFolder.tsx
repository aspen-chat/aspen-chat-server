import { FOLDER_COLORS, type FolderColor, type RailFolder } from "@aspen/protocol";
import { CaretRightIcon, FolderOpenIcon } from "@phosphor-icons/react";
import { useState, type ReactNode, type RefObject } from "react";
import {
  Button,
  Dialog,
  Form,
  Input,
  Label,
  Menu,
  MenuItem,
  Modal,
  ModalOverlay,
  Popover,
  SubmenuTrigger,
  TextField,
} from "react-aria-components";
import { SourceScope } from "@/api/deployments";
import type { Source } from "@/api/everywhere";
import { fieldClass, inputClass, labelClass, primaryButtonClass } from "@/features/auth/styles";
import { Avatar } from "@/features/communities/Avatar";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { FOLDER_TINT, folderName } from "@/features/communities/folders";

/** A community a folder shows, drawn in its own deployment's scope so its icon loads. */
export interface FolderMember {
  readonly key: string;
  readonly name: string;
  readonly icon: string | null | undefined;
  readonly source: Source;
}

/**
 * A folder's face in the rail. Closed, it is a tile in the folder's tint showing its first four
 * communities' icons two by two, as an iPhone's folders do; open, it is the tile with an open
 * folder on it, heading the band its communities stand on below.
 */
export function FolderTile({
  folder,
  members,
}: {
  folder: RailFolder;
  members: readonly FolderMember[];
}) {
  const tint = FOLDER_TINT[folder.color];
  if (folder.open) {
    return (
      <span
        className={`flex h-12 w-12 items-center justify-center rounded-2xl text-ink-muted ${tint}`}
      >
        <FolderOpenIcon size={22} aria-hidden="true" />
      </span>
    );
  }
  return (
    <span
      className={`grid h-12 w-12 grid-cols-2 place-items-center gap-0.5 rounded-2xl p-1 ${tint}`}
    >
      {members.slice(0, 4).map((member) => (
        <SourceScope key={member.key} source={member.source}>
          <Avatar name={member.name} iconId={member.icon} size="xs" />
        </SourceScope>
      ))}
    </span>
  );
}

const popoverClass = "w-48 rounded-md border border-line bg-surface-raised p-1 shadow-lg";
const itemClass = "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover";
const parentClass = itemClass + " flex items-center gap-2 open:bg-surface-hover";

/**
 * What can be done to a folder: rename it, tint it, or ungroup it, its communities standing
 * where it stood. It opens beside the folder, from a right click or its options button.
 */
export function FolderMenu({
  folder,
  anchorRef,
  isOpen,
  onOpenChange,
  onRename,
  onColor,
  onUngroup,
}: {
  folder: RailFolder;
  anchorRef: RefObject<HTMLElement | null>;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  onRename: () => void;
  onColor: (color: FolderColor) => void;
  onUngroup: () => void;
}) {
  const m = useMessages();
  const label = format(m.folders.options, { name: folderName(m, folder) });
  return (
    <Popover
      triggerRef={anchorRef}
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      placement="end top"
      className={popoverClass}
    >
      <Dialog aria-label={label} className="outline-none">
        <Menu
          aria-label={label}
          className="outline-none"
          onAction={(key) => {
            onOpenChange(false);
            if (key === "rename") {
              onRename();
            } else if (key === "ungroup") {
              onUngroup();
            }
          }}
        >
          <MenuItem id="rename" className={itemClass}>
            {m.folders.rename}
          </MenuItem>
          <SubmenuTrigger>
            <MenuItem id="color" className={parentClass}>
              <span className="flex-1">{m.folders.color}</span>
              <CaretRightIcon size={12} aria-hidden="true" className="rtl:-scale-x-100" />
            </MenuItem>
            <Popover className={popoverClass} placement="end top">
              <Menu
                aria-label={m.folders.color}
                className="outline-none"
                selectionMode="single"
                selectedKeys={[folder.color]}
                onAction={(key) => {
                  onOpenChange(false);
                  const color = FOLDER_COLORS.find((c) => c === key);
                  if (color !== undefined) {
                    onColor(color);
                  }
                }}
              >
                {FOLDER_COLORS.map((color) => (
                  <MenuItem
                    key={color}
                    id={color}
                    className={
                      itemClass +
                      " flex items-center gap-2 selected:font-medium selected:text-accent"
                    }
                  >
                    <span
                      aria-hidden="true"
                      className={`h-3.5 w-3.5 rounded-full border border-line ${FOLDER_TINT[color]}`}
                    />
                    {m.folders.colors[color]}
                  </MenuItem>
                ))}
              </Menu>
            </Popover>
          </SubmenuTrigger>
          <MenuItem id="ungroup" className={itemClass}>
            {m.folders.ungroup}
          </MenuItem>
        </Menu>
      </Dialog>
    </Popover>
  );
}

/** Names a folder; an empty name leaves it called by the plain word. */
export function RenameFolderDialog({
  folder,
  isOpen,
  onOpenChange,
  onSave,
}: {
  folder: RailFolder;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (name: string) => void;
}) {
  const m = useMessages();
  const [name, setName] = useState(folder.name);
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{m.folders.renameHeading}</DialogHeading>
          <Form
            onSubmit={(event) => {
              event.preventDefault();
              onSave(name.trim());
              onOpenChange(false);
            }}
            className="flex flex-col gap-3"
          >
            <TextField value={name} onChange={setName} autoFocus className={fieldClass}>
              <Label className={labelClass}>{m.folders.nameLabel}</Label>
              <Input placeholder={m.folders.untitled} className={inputClass} />
            </TextField>
            <Button
              type="submit"
              isDisabled={name.trim() === folder.name}
              className={primaryButtonClass + " self-start"}
            >
              {m.folders.save}
            </Button>
          </Form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** Keeps a folder's options reachable from the keyboard, beside its drag handle. */
export function FolderOptionsButton({
  label,
  onPress,
  children,
}: {
  label: string;
  onPress: () => void;
  children: ReactNode;
}) {
  return (
    <Button
      aria-label={label}
      onPress={onPress}
      className="absolute -end-1 -top-1 rounded-full border border-line bg-surface-raised p-0.5 text-ink-faint opacity-0 outline-none focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      {children}
    </Button>
  );
}
