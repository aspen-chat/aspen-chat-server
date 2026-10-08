import type { ReactNode, Ref } from "react";
import { Button, type ButtonProps } from "react-aria-components";
import { Tooltip } from "@/features/layout/Tooltip";

/**
 * A control drawn as its icon and named by a tooltip; or, `labelled`, drawn as its icon with
 * its name after it, a row of a list such as a touch screen's message actions, where the name
 * shown names it and no tooltip is needed.
 */
export function IconAction({
  label,
  labelled = false,
  icon,
  className,
  ref,
  ...props
}: Omit<ButtonProps, "children" | "className" | "aria-label"> & {
  label: string;
  labelled?: boolean;
  icon: ReactNode;
  className: string;
  ref?: Ref<HTMLButtonElement>;
}) {
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
