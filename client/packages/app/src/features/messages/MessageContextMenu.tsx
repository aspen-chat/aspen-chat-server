import { ArrowSquareOutIcon, CopyIcon, DownloadSimpleIcon, LinkIcon } from "@phosphor-icons/react";
import type { ComponentProps, RefObject } from "react";
import { Menu, MenuSection } from "react-aria-components";
import { copyText } from "@/features/layout/clipboard";
import { ContextMenu } from "@/features/layout/ContextMenu";
import type { ContextMenuState } from "@/features/layout/contextMenuState";
import { ActionsAsMenuItems } from "@/features/layout/actionsAsMenuItems";
import { MenuAction, MenuRule, menuClass } from "@/features/layout/menu";
import { toast } from "@/features/layout/toast";
import { QuickReactions } from "@/features/messages/MessageActionSheet";
import { MessageActions, type MessageSheet } from "@/features/messages/MessageActions";
import {
  UnreadableFile,
  canCopyPictures,
  copyPicture,
  saveFrom,
} from "@/features/messages/mediaFile";
import type { UnderPointer } from "@/features/messages/underPointer";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** Opens `url` in a new tab, or the system's browser, as a link in a message would. */
function openElsewhere(url: string) {
  const link = document.createElement("a");
  link.href = url;
  link.target = "_blank";
  link.rel = "noreferrer noopener";
  link.click();
}

/**
 * A message's actions, opened by right-clicking it or by the last button of the row over it
 * (`button`), beside which it then opens: the reader's quick reactions, where they may react,
 * with Add a reaction beside them; what was right-clicked, when it is an attachment or a link's
 * picture (open it, save it, copy a picture, copy its address) or a link (open it, copy its
 * address); and then every action the message has, by name, through the same `MessageActions`,
 * those the row offers and those it leaves to the menu. What an action opens goes through
 * `open`, outside the menu, which closes as it is chosen.
 */
export function MessageContextMenu({
  menu,
  under,
  canReact,
  actions,
  open,
  button,
}: {
  menu: ContextMenuState;
  under: UnderPointer;
  canReact: boolean;
  actions: Omit<ComponentProps<typeof MessageActions>, "open" | "onDone">;
  open: (sheet: MessageSheet) => void;
  button: RefObject<HTMLElement | null>;
}) {
  const m = useMessages();
  const close = () => {
    menu.onOpenChange(false);
  };
  const copy = (url: string) => {
    void copyText(url, document.body).then((copied) => {
      if (copied) {
        toast(m.contextMenu.copiedAddress);
      }
    });
  };
  // Says why a file could not be read: the storage's, which its administrators set up, or else
  // what went wrong.
  const failed = (failure: unknown) => {
    toast(
      failure instanceof UnreadableFile
        ? m.contextMenu.unreadableFile
        : format(m.contextMenu.fileFailed, {
            problem: failure instanceof Error ? failure.message : String(failure),
          }),
    );
  };
  return (
    <ContextMenu menu={menu} label={m.messageActionsLabel} button={button}>
      {canReact && (
        <QuickReactions
          messageId={actions.messageId}
          communityId={actions.communityId}
          compact
          onDone={close}
          onMore={() => {
            close();
            open("react");
          }}
        />
      )}
      <Menu
        aria-label={m.messageActionsLabel}
        autoFocus={menu.byKeyboard ? "first" : false}
        onAction={close}
        className={menuClass}
      >
        {under !== null && (
          <>
            <MenuSection
              aria-label={under.kind === "media" ? m.contextMenu.attachment : m.contextMenu.link}
            >
              <MenuAction
                label={under.kind === "media" ? m.contextMenu.openOriginal : m.contextMenu.openLink}
                icon={<ArrowSquareOutIcon aria-hidden="true" />}
                onAction={() => {
                  if (under.kind === "link") {
                    under.anchor.click();
                  } else {
                    openElsewhere(under.url);
                  }
                }}
              />
              {under.kind === "media" && (
                <MenuAction
                  label={under.picture ? m.contextMenu.savePicture : m.contextMenu.saveFile}
                  icon={<DownloadSimpleIcon aria-hidden="true" />}
                  onAction={() => {
                    saveFrom(under.url, under.name).catch(failed);
                  }}
                />
              )}
              {under.kind === "media" && under.picture && canCopyPictures() && (
                <MenuAction
                  label={m.contextMenu.copyPicture}
                  icon={<CopyIcon aria-hidden="true" />}
                  onAction={() => {
                    copyPicture(under.url).then(() => {
                      toast(m.contextMenu.copiedPicture);
                    }, failed);
                  }}
                />
              )}
              <MenuAction
                label={
                  under.kind === "media"
                    ? m.contextMenu.copyFileAddress
                    : m.contextMenu.copyLinkAddress
                }
                icon={<LinkIcon aria-hidden="true" />}
                onAction={() => {
                  copy(under.url);
                }}
              />
            </MenuSection>
            <MenuRule />
          </>
        )}
        <ActionsAsMenuItems value>
          <MessageActions {...actions} open={open} onDone={close} />
        </ActionsAsMenuItems>
      </Menu>
    </ContextMenu>
  );
}
