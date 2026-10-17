import { useContext, type ReactNode, type Ref } from "react";
import { Button, type ButtonProps } from "react-aria-components";
import { ActionsAsMenuItems } from "@/features/layout/actionsAsMenuItems";
import { MenuAction } from "@/features/layout/menu";
import { Tooltip } from "@/features/layout/Tooltip";

/**
 * A control drawn as its icon and named by a tooltip; or, `labelled`, drawn as its icon with
 * its name after it, a row of a list such as a touch screen's message actions, where the name
 * shown names it and no tooltip is needed. Inside `ActionsAsMenuItems` it is a menu's item
 * instead, drawn as the menu draws them, `danger` among them in the danger colour; it has no
 * element of its own there for `ref` to hold.
 */
export function IconAction({
  label,
  labelled = false,
  danger = false,
  icon,
  className,
  ref,
  ...props
}: Omit<ButtonProps, "children" | "className" | "aria-label" | "onPress"> & {
  label: string;
  onPress?: () => void;
  labelled?: boolean;
  /** Whether it does what cannot be undone, which a menu marks; elsewhere `className` does. */
  danger?: boolean;
  icon: ReactNode;
  className: string;
  ref?: Ref<HTMLButtonElement>;
}) {
  const inMenu = useContext(ActionsAsMenuItems);
  if (inMenu) {
    const { onPress, isDisabled } = props;
    return (
      <MenuAction
        label={label}
        icon={icon}
        danger={danger}
        {...(isDisabled === undefined ? {} : { isDisabled })}
        {...(onPress === undefined ? {} : { onAction: onPress })}
      />
    );
  }
  if (labelled) {
    return (
      <Button ref={ref} {...props} className={className}>
        {icon}
        <span>{label}</span>
      </Button>
    );
  }
  return (
    <Tooltip text={label}>
      <Button ref={ref} {...props} aria-label={label} className={className}>
        {icon}
      </Button>
    </Tooltip>
  );
}
