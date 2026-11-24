import { CopyIcon, DotsThreeIcon, DownloadSimpleIcon, EyeIcon } from "@phosphor-icons/react";
import { useContext, type RefObject } from "react";
import { Dialog, Menu, Popover } from "react-aria-components";
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

/**
 * A picture's own actions, opened by a finger's long press on it: a popover beneath the picture
 * (above it when there is no room beneath) offering to view it in the gallery, download it, and,
 * where the clipboard takes a picture, copy it, and last, where the message has a sheet of its
 * actions, More options, which opens that sheet. Every action is on the original picture
 * (`Picture.src`), never on the smaller copy shown inline, but copying one whose original is
 * served to be saved, which copies what is shown of it (`Picture.shown`). A pointer's right click offers the
 * same in the message's context menu (`MessageContextMenu`). Built from when it first opens, as
 * every picture of every row has one.
 */
export function PictureMenu({
  picture,
  triggerRef,
  isOpen,
  onOpenChange,
  onView,
}: {
  picture: Picture;
  triggerRef: RefObject<HTMLElement | null>;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  /** Opens the gallery on the picture. */
  onView: () => void;
}) {
  const m = useMessages();
  const openMessageActions = useContext(OpenMessageActionsContext);
  const failed = (failure: unknown) => {
    toastFileFailure(m, failure);
  };
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
        <Dialog aria-label={m.pictureMenu.label} className="outline-none">
          <Menu
            aria-label={m.pictureMenu.label}
            onAction={() => {
              onOpenChange(false);
            }}
            className={menuClass}
          >
            <MenuAction
              label={m.contextMenu.viewImage}
              icon={<EyeIcon aria-hidden="true" />}
              onAction={onView}
            />
            <MenuAction
              label={m.contextMenu.downloadImage}
              icon={<DownloadSimpleIcon aria-hidden="true" />}
              onAction={() => {
                saveFrom(picture.src, picture.name).catch(failed);
              }}
            />
            {canCopyPictures() && (
              <MenuAction
                label={m.contextMenu.copyPicture}
                icon={<CopyIcon aria-hidden="true" />}
                onAction={() => {
                  copyPicture(shownSrc(picture)).then(() => {
                    toast(m.contextMenu.copiedPicture);
                  }, failed);
                }}
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
