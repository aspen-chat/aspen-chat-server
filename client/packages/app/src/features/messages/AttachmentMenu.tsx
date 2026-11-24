import {
  ArrowSquareOutIcon,
  CopyIcon,
  DotsThreeIcon,
  DownloadSimpleIcon,
  EyeIcon,
  LinkIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { useContext, type RefObject } from "react";
import { Dialog, Menu, Popover } from "react-aria-components";
import { copyText } from "@/features/layout/clipboard";
import { MenuAction, menuClass, menuPopoverClass } from "@/features/layout/menu";
import { OnceOpen } from "@/features/layout/OnceOpen";
import { toast } from "@/features/layout/toast";
import { shownSrc, type Picture } from "@/features/messages/images";
import { OpenMessageActionsContext } from "@/features/messages/messageActionsContext";
import {
  canCopyPictures,
  copyPicture,
  saveFrom,
  toastFileFailure,
} from "@/features/messages/mediaFile";
import { useMessages } from "@/i18n/context";

/** What an `AttachmentMenu` is for: a picture, which opens the gallery, or any other file. */
export type MenuTarget =
  | { readonly kind: "picture"; readonly picture: Picture; readonly onView: () => void }
  | { readonly kind: "file"; readonly url: string; readonly name: string };

/**
 * An attachment's own actions, opened by a finger's long press on it: a popover beneath it
 * (above it when there is no room beneath). A picture offers to view it in the gallery, download
 * it, and, where the clipboard takes a picture, copy it; any other file to open the original, save
 * it, and copy its address. With `onRemove`, Remove attachment follows, which asks before taking
 * it off the message (`MessageItem`). Last, where the message has a sheet of its actions, More
 * options opens that sheet. Every action is on the original file (`Picture.src`), never on the
 * smaller copy shown inline, but copying a picture whose original is served to be saved, which
 * copies what is shown of it (`Picture.shown`). A pointer's right click offers the same in the
 * message's context menu (`MessageContextMenu`). Built from when it first opens, as every
 * attachment of every row has one.
 */
export function AttachmentMenu({
  target,
  triggerRef,
  isOpen,
  onOpenChange,
  onRemove,
}: {
  target: MenuTarget;
  triggerRef: RefObject<HTMLElement | null>;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  /** Asks to take the attachment off the message, for its author and those who manage messages. */
  onRemove?: () => void;
}) {
  const m = useMessages();
  const openMessageActions = useContext(OpenMessageActionsContext);
  const failed = (failure: unknown) => {
    toastFileFailure(m, failure);
  };
  const label = target.kind === "picture" ? m.pictureMenu.label : m.attachmentMenu.label;
  return (
    <OnceOpen isOpen={isOpen}>
      <Popover
        triggerRef={triggerRef}
        isOpen={isOpen}
        onOpenChange={onOpenChange}
        placement="bottom"
        offset={8}
        className={menuPopoverClass}
      >
        <Dialog aria-label={label} className="outline-none">
          <Menu
            aria-label={label}
            onAction={() => {
              onOpenChange(false);
            }}
            className={menuClass}
          >
            {target.kind === "picture" ? (
              <>
                <MenuAction
                  label={m.contextMenu.viewImage}
                  icon={<EyeIcon aria-hidden="true" />}
                  onAction={target.onView}
                />
                <MenuAction
                  label={m.contextMenu.downloadImage}
                  icon={<DownloadSimpleIcon aria-hidden="true" />}
                  onAction={() => {
                    saveFrom(target.picture.src, target.picture.name).catch(failed);
                  }}
                />
                {canCopyPictures() && (
                  <MenuAction
                    label={m.contextMenu.copyPicture}
                    icon={<CopyIcon aria-hidden="true" />}
                    onAction={() => {
                      copyPicture(shownSrc(target.picture)).then(() => {
                        toast(m.contextMenu.copiedPicture);
                      }, failed);
                    }}
                  />
                )}
              </>
            ) : (
              <>
                <MenuAction
                  label={m.contextMenu.openOriginal}
                  icon={<ArrowSquareOutIcon aria-hidden="true" />}
                  href={target.url}
                  target="_blank"
                  rel="noreferrer noopener"
                />
                <MenuAction
                  label={m.contextMenu.saveFile}
                  icon={<DownloadSimpleIcon aria-hidden="true" />}
                  onAction={() => {
                    saveFrom(target.url, target.name).catch(failed);
                  }}
                />
                <MenuAction
                  label={m.contextMenu.copyFileAddress}
                  icon={<LinkIcon aria-hidden="true" />}
                  onAction={() => {
                    void copyText(target.url, document.body).then((copied) => {
                      if (copied) {
                        toast(m.contextMenu.copiedAddress);
                      }
                    });
                  }}
                />
              </>
            )}
            {onRemove !== undefined && (
              <MenuAction
                label={m.contextMenu.removeAttachment}
                icon={<TrashIcon aria-hidden="true" />}
                danger
                onAction={onRemove}
              />
            )}
            {openMessageActions !== null && (
              <MenuAction
                label={m.pictureMenu.more}
                icon={<DotsThreeIcon aria-hidden="true" />}
                onAction={openMessageActions}
              />
            )}
          </Menu>
        </Dialog>
      </Popover>
    </OnceOpen>
  );
}
