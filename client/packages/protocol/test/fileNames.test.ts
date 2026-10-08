import { describe, expect, it } from "vitest";
import { plainFileName } from "../src/fileNames";

describe("plainFileName", () => {
  it("drops the characters that change how a name reads", () => {
    expect(plainFileName("photo‮gpj.exe")).toBe("photogpj.exe");
    expect(plainFileName("a⁦b⁩c​d‏e­f")).toBe("abcdef");
    expect(plainFileName("line\nbreak\u0000.txt")).toBe("linebreak.txt");
  });

  it("keeps names in every script", () => {
    for (const name of ["résumé.pdf", "写真.png", "صورة.jpg", "фото 1.jpeg"]) {
      expect(plainFileName(name)).toBe(name);
    }
  });
});
