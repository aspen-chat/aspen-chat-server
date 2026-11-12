import type { Nodes } from "mdast";
import { describe, expect, it } from "vitest";
import spec from "../../../../../../spec/spoiler_vectors.json";
import { maskSpoilers, spoilerTree } from "./maskSpoilers";
import { opening } from "./remarkSpoilers";

/** What stands for a spoiler in a vector's `shown`, and for code in `shown` and `hidden`. */
const SPOILER = "▮";
const CODE = "⌷";
/** The nodes that stand in a line of text and hold more of it. */
const INLINE = new Set(["emphasis", "strong", "delete", "link"]);

/** What a vector says of its `content`: what is shown of it, and what each spoiler hides. */
function read(content: string): { shown: string; hidden: string[] } {
  const tree = spoilerTree(content);
  if (tree === null) {
    return { shown: spaced(content), hidden: [] };
  }
  let shown = "";
  const hidden: string[] = [];
  const walk = (node: Nodes, spoiled: boolean) => {
    const add = (text: string) => {
      if (spoiled) {
        hidden[hidden.length - 1] = (hidden.at(-1) ?? "") + text;
      } else {
        shown += text;
      }
    };
    if ((node.type as string) === "spoiler" && !spoiled) {
      shown += SPOILER;
      hidden.push("");
    }
    if (node.type === "text") {
      add(node.value);
    } else if (node.type === "inlineCode" || node.type === "code") {
      add(CODE);
    } else if (!("children" in node)) {
      add(" ");
    } else {
      for (const child of node.children) {
        walk(child, spoiled || (node.type as string) === "spoiler");
      }
      if (!INLINE.has(node.type) && (node.type as string) !== "spoiler" && node !== tree) {
        shown += "\n";
      }
    }
  };
  walk(tree, false);
  return { shown: spaced(shown), hidden: hidden.map(spaced) };
}

function spaced(text: string): string {
  return text
    .split(/[ \t\n\r\f]+/)
    .filter((word) => word !== "")
    .join(" ");
}

describe("the spoilers of a message", () => {
  it.each(spec.vectors)("are where the vector says: $name", ({ content, shown, hidden }) => {
    expect(read(content)).toEqual({ shown, hidden });
  });
});

describe("opening", () => {
  const pipes = { open: "||", close: "||" };
  const reddit = { open: ">!", close: "!<" };

  it("passes by markers that hold nothing or only blank text", () => {
    expect(opening("||||a||", "||||a||", pipes)).toEqual({ openAt: 2, closeAt: 5 });
    expect(opening("|| || ||a||", "|| || ||a||", pipes)).toEqual({ openAt: 6, closeAt: 9 });
    expect(opening("|| || ||a", "|| || ||a", pipes)).toEqual({ openAt: 6, closeAt: null });
    expect(opening("|| |||| ||", "|| |||| ||", pipes)).toBeNull();
    expect(opening(">!!<>! !<>!a!<", ">!!<>! !<>!a!<", reddit)).toEqual({ openAt: 9, closeAt: 12 });
  });

  // Were each step to start the run over, these would take hours.
  it("reads a long run of markers once", () => {
    for (const run of ["|".repeat(4_000_001), "|| ".repeat(1_000_000), "||\t".repeat(1_000_000)]) {
      const found = opening(`${run}||a||`, `${run}||a||`, pipes);
      expect(found?.closeAt).toBe(`${run}||a||`.length - 2);
    }
    const run = ">!!<".repeat(1_000_000);
    expect(opening(run, run, reddit)).toBeNull();
    expect(spoilerTree("|".repeat(20_001))).not.toBeNull();
  });
});

describe("maskSpoilers", () => {
  const masked = (content: string) => maskSpoilers(content, "[spoiler]");

  it("writes each spoiler as the word, in either spelling, and leaves the rest as written", () => {
    expect(masked("the killer is ||**the butler**|| _obviously_")).toBe(
      "the killer is [spoiler] _obviously_",
    );
    expect(masked(">!one!< and **a ||two|| b**\n\n# ||three||")).toBe(
      "[spoiler] and **a [spoiler] b**\n\n# [spoiler]",
    );
    expect(masked("||a||||b||")).toBe("[spoiler][spoiler]");
  });

  it("hides the code and the markup a spoiler holds, and no marker in code", () => {
    expect(masked("a || `code` **b** || c")).toBe("a [spoiler] c");
    expect(masked("`||a||`\n\n```\n||b||\n```")).toBe("`||a||`\n\n```\n||b||\n```");
  });

  it("leaves a body with no spoiler exactly as it was", () => {
    for (const content of [
      "plain",
      "a || b",
      "|| || and ||||",
      "> !a b",
      "a\\|b &amp; c",
      "\\|\\|a\\|\\| b",
      "&#124;&vert;a|&#x7c; b",
      "&gt;!a!&lt; and \\>!b!<",
      "",
    ]) {
      expect(masked(content)).toBe(content);
    }
  });

  it("finds a spoiler after markers that hold nothing, a quote's own >, and a cell's pipes", () => {
    expect(masked("||||secret|| and || || ||real||")).toBe("||[spoiler] and || || [spoiler]");
    expect(masked("x\n\n> !a\n> b!< c")).toBe("x\n\n[spoiler] c");
    expect(masked("| a |\n|---|\n| \\|\\|x\\|\\| y |")).toBe("| a |\n|---|\n| [spoiler] y |");
  });

  it("writes the text a spoiler was cut from as it is shown", () => {
    expect(masked("1 \\* 2 ||a|| 3 \\* 4\n\n5 \\* 6")).toBe("1 * 2 [spoiler] 3 * 4\n\n5 \\* 6");
  });

  it("hides a spoiler around an address, a named link, and a definition's", () => {
    expect(masked("see ||https://example.org|| now")).toBe("see [spoiler] now");
    expect(masked("[||named||](https://named.example)")).toBe("[[spoiler]](https://named.example)");
    expect(masked('[x]: https://x.example "||s||"')).toBe('[x]: https://x.example "[spoiler]"');
  });

  it("hides nothing in a body shown as it was written", () => {
    const quoted = `${">".repeat(33)}||a||`;
    expect(masked(quoted)).toBe(quoted);
  });

  it.each(spec.vectors)("writes one word for each spoiler: $name", ({ content, hidden }) => {
    expect(maskSpoilers(content, SPOILER).split(SPOILER).length - 1).toBe(hidden.length);
  });
});
