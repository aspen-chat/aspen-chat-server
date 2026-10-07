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

/**
 * `nodes` joined as a list in `locale`'s words ("A, B, and C"), for a placeholder of
 * `formatNodes`: the language decides the separators and their order.
 */
export function listNodes(locale: string, nodes: readonly ReactNode[]): ReactNode {
  const parts = new Intl.ListFormat(locale, { style: "long", type: "conjunction" }).formatToParts(
    nodes.map((_, index) => String(index)),
  );
  return parts.map((part, index) => (
    <Fragment key={index}>
      {part.type === "element" ? nodes[Number(part.value)] : part.value}
    </Fragment>
  ));
}
