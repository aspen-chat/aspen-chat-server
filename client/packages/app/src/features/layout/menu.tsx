import { CaretRightIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { MenuItem, Separator, type MenuItemProps } from "react-aria-components";

/** Every menu's popover: raised over what it opens from, a little wider than its longest item. */
export const menuPopoverClass =
  "min-w-56 max-w-80 rounded-md border border-line bg-surface-raised p-1 shadow-lg";
export const menuClass = "max-h-[inherit] overflow-y-auto outline-none";
export const menuItemClass =
  "flex cursor-default items-center gap-2 rounded px-2 py-1 text-sm text-ink outline-none " +
  "focus:bg-surface-hover disabled:text-ink-faint";
/** An item that opens a submenu: its label, what is in force, and an arrow. */
export const menuParentClass = menuItemClass + " open:bg-surface-hover";
export const menuHeaderClass = "px-2 pt-1 pb-0.5 text-xs font-semibold text-ink-faint";
/** A choice among several, marked when it is the one in force. */
export const menuCheckClass = "selected:font-medium selected:text-accent";

/** The rule between a menu's groups of items. */
export function MenuRule() {
  return <Separator className="my-1 h-px bg-line" />;
}

/**
 * An item of a menu drawn as its icon and its name, the icon at the menu's size whatever size
 * it was drawn for, and in the danger colour for what cannot be undone.
 */
export function MenuAction({
  label,
  icon,
  danger = false,
  ...props
}: Omit<MenuItemProps, "children" | "className" | "textValue"> & {
  label: string;
  icon: ReactNode;
  danger?: boolean;
}) {
  return (
    <MenuItem
      {...props}
      textValue={label}
      className={menuItemClass + (danger ? " text-danger" : "")}
    >
      <span
        className={"flex shrink-0 [&>svg]:h-3.5 [&>svg]:w-3.5 " + (danger ? "" : "text-ink-muted")}
      >
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
    </MenuItem>
  );
}

/** A submenu's item: its label, what is in force there when given, and an arrow to the submenu. */
export function ParentLabel({ children, detail }: { children: ReactNode; detail?: string }) {
  return (
    <>
      <span className="min-w-0 flex-1 truncate">{children}</span>
      {detail !== undefined && <span className="shrink-0 text-xs text-ink-faint">{detail}</span>}
      <CaretRightIcon
        size={12}
        aria-hidden="true"
        className="shrink-0 text-ink-faint rtl:-scale-x-100"
      />
    </>
  );
}
