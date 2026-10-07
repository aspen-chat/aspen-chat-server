import { describe, expect, it } from "vitest";
import { typingSentence } from "./typingSentence";

describe("typingSentence", () => {
  it("puts each name apart from the words around it", () => {
    expect(typingSentence("{names} is typing…", "en", 1)).toEqual([
      { name: 0 },
      { text: " is typing…" },
    ]);
    expect(typingSentence("{names} are typing…", "en", 3)).toEqual([
      { name: 0 },
      { text: ", " },
      { name: 1 },
      { text: ", and " },
      { name: 2 },
      { text: " are typing…" },
    ]);
  });

  it("follows the language's list and the template's order", () => {
    expect(typingSentence("Es schreiben {names}…", "de", 2)).toEqual([
      { text: "Es schreiben " },
      { name: 0 },
      { text: " und " },
      { name: 1 },
      { text: "…" },
    ]);
  });
});
