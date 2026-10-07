import { Fragment, type ReactNode } from "react";

/**
 * `format` for placeholders that stand for elements rather than text, such as a person's chip:
 * the template's text around them stays text, so a translation may put the element anywhere.
 */
export function formatNodes(template: string, values: Record<string, ReactNode>): ReactNode {
  const parts = template.split(/(\{\w+\})/);
  return parts.map((part, index) => {
    const key = /^\{(\w+)\}$/.exec(part)?.[1];
    return (
      <Fragment key={index}>{key !== undefined && key in values ? values[key] : part}</Fragment>
    );
  });
}
