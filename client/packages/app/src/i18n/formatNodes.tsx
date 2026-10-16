import { Fragment, type ReactNode } from "react";
import { formatParts } from "./messages";

/**
 * `format` for placeholders that stand for elements rather than text, such as a person's chip:
 * the template's text around them stays text, so a translation may put the element anywhere.
 */
export function formatNodes(template: string, values: Record<string, ReactNode>): ReactNode {
  const formatted = formatParts<ReactNode>(template, values);
  if (!Array.isArray(formatted)) {
    return formatted;
  }
  return formatted.map((part, index) => <Fragment key={index}>{part}</Fragment>);
}
