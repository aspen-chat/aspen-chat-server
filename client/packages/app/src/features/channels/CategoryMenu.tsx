import type { Category } from "@aspen/protocol";
import {
  ArrowsInLineVerticalIcon,
  ArrowsOutLineVerticalIcon,
  CaretDownIcon,
  CaretRightIcon,
  CheckCircleIcon,
  LockKeyIcon,
  PencilSimpleIcon,
  PlusIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { useState } from "react";
import { Menu } from "react-aria-components";
import {
  useAnyUnread,
  useCan,
  useCategoryNotificationLevel,
  useCollapsed,
  useSync,
} from "@/api/hooks";
import { CategoryMuteSubmenu, NotificationsSubmenu } from "@/features/channels/muteMenus";
import { problemText } from "@/api/problemText";
import { AddToCategoryModal } from "@/features/channels/AddToCategoryDialog";
import { DeleteCategoryDialog, RenameCategoryDialog } from "@/features/channels/CategoryDialogs";
import { AccessDialog } from "@/features/community-settings/AccessDialog";
import { ContextMenu } from "@/features/layout/ContextMenu";
import type { ContextMenuState } from "@/features/layout/contextMenuState";
import { CopyIdMenuItem } from "@/features/layout/CopyId";
import { MenuAction, MenuRule, menuClass, menuItemClass } from "@/features/layout/menu";
import { toast } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** Says why something failed, in a toast, as a menu that has closed cannot. */
function sayFailure(failure: unknown) {
  toast(problemText(failure));
}

/** What an item of a category's menu opens outside it, which closes as the item is chosen. */
type CategoryDialog = "add" | "access" | "rename" | "delete";

/**
 * What the reader can do to a category from its heading, opened by right-clicking it: fold it
 * away or back, as the heading does, or every category of the community at once; mute it, and
 * with it its channels that have no mute of their own; choose what its channels without a
 * setting of their own tell of; mark its channels read, while one is unread; and, for
 * those who may, add a channel to it (Manage channels), or set who can use it, rename it, or
 * delete it (Manage categories).
 */
export function CategoryMenu({
  menu,
  category,
  channelIds,
}: {
  menu: ContextMenuState;
  category: Category;
  /** Every channel in it, folded away or not. */
  channelIds: readonly string[];
}) {
  const m = useMessages();
  const sync = useSync();
  const collapsed = useCollapsed(category.id);
  const unread = useAnyUnread(channelIds);
  const addChannels = useCan(category.community, "manageChannels");
  const manage = useCan(category.community, "manageCategories");
  const levels = useCategoryNotificationLevel(category.id, category.community);
  const [dialog, setDialog] = useState<CategoryDialog | null>(null);
  const close = () => {
    menu.onOpenChange(false);
  };
  const label = format(m.mute.options, { name: category.name });
  const open = (next: CategoryDialog) => () => {
    setDialog(next);
  };
  const dialogProps = (which: CategoryDialog) => ({
    isOpen: dialog === which,
    onOpenChange: (isOpen: boolean) => {
      if (!isOpen) {
        setDialog(null);
      }
    },
  });
  return (
    <>
      <ContextMenu menu={menu} label={label}>
        <Menu
          aria-label={label}
          autoFocus={menu.byKeyboard ? "first" : false}
          onAction={() => {
            menu.onOpenChange(false);
          }}
          className={menuClass}
        >
          <MenuAction
            label={collapsed ? m.contextMenu.expandCategory : m.contextMenu.collapseCategory}
            icon={
              collapsed ? (
                <CaretRightIcon aria-hidden="true" className="rtl:-scale-x-100" />
              ) : (
                <CaretDownIcon aria-hidden="true" />
              )
            }
            onAction={() => {
              void sync.setCategoryCollapsed(category.id, !collapsed).catch(() => undefined);
            }}
          />
          <MenuAction
            label={m.contextMenu.collapseAll}
            icon={<ArrowsInLineVerticalIcon aria-hidden="true" />}
            onAction={() => {
              sync.setAllCategoriesCollapsed(category.community, true).catch(sayFailure);
            }}
          />
          <MenuAction
            label={m.contextMenu.expandAll}
            icon={<ArrowsOutLineVerticalIcon aria-hidden="true" />}
            onAction={() => {
              sync.setAllCategoriesCollapsed(category.community, false).catch(sayFailure);
            }}
          />
          {unread && (
            <MenuAction
              label={m.contextMenu.markRead}
              icon={<CheckCircleIcon aria-hidden="true" />}
              onAction={() => {
                sync.markChannelsRead(channelIds).catch(sayFailure);
              }}
            />
          )}
          <CategoryMuteSubmenu categoryId={category.id} close={close} />
          <NotificationsSubmenu
            own={levels.own}
            inherited={levels.inherited}
            choose={(level) => sync.setCategoryNotifications(category.id, level)}
            close={close}
          />
          {(addChannels || manage) && <MenuRule />}
          {addChannels && (
            <MenuAction
              label={m.contextMenu.addChannel}
              icon={<PlusIcon aria-hidden="true" />}
              onAction={open("add")}
            />
          )}
          {manage && (
            <>
              <MenuAction
                label={m.access.open}
                icon={<LockKeyIcon aria-hidden="true" />}
                onAction={open("access")}
              />
              <MenuAction
                label={m.contextMenu.renameCategory}
                icon={<PencilSimpleIcon aria-hidden="true" />}
                onAction={open("rename")}
              />
              <MenuAction
                label={m.contextMenu.deleteCategory}
                icon={<TrashIcon aria-hidden="true" />}
                danger
                onAction={open("delete")}
              />
            </>
          )}
          <CopyIdMenuItem id={category.id} thing="category" className={menuItemClass} />
        </Menu>
      </ContextMenu>
      {dialog === "add" && <AddToCategoryModal category={category} {...dialogProps("add")} />}
      {dialog === "access" && (
        <AccessDialog
          target={{
            kind: "category",
            id: category.id,
            name: category.name,
            communityId: category.community,
          }}
          {...dialogProps("access")}
        />
      )}
      {dialog === "rename" && (
        <RenameCategoryDialog category={category} {...dialogProps("rename")} />
      )}
      {dialog === "delete" && (
        <DeleteCategoryDialog category={category} {...dialogProps("delete")} />
      )}
    </>
  );
}
