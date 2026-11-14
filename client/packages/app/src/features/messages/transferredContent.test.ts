import { describe, expect, it } from "vitest";
import { timestampedName, transferredContent } from "./transferredContent";

/** A `DataTransfer` holding `files` and `text`, as much of one as the reader reads. */
function carrying(files: File[], text = ""): DataTransfer {
  return {
    files: Object.assign([...files], { item: (i: number) => files[i] ?? null }),
    items: [],
    getData: (type: string) => (type === "text/plain" ? text : ""),
  } as unknown as DataTransfer;
}

const named = () => "img_named.png";

describe("transferredContent", () => {
  it("names image data that came from no file", () => {
    const { files } = transferredContent(
      carrying([new File(["x"], "image.png", { type: "image/png" })]),
      named,
    );
    expect(files.map((file) => [file.name, file.type])).toEqual([["img_named.png", "image/png"]]);
  });

  it("keeps the name of a file that has one", () => {
    const { files } = transferredContent(
      carrying([new File(["x"], "holiday.png", { type: "image/png" })]),
      named,
    );
    expect(files[0]?.name).toBe("holiday.png");
  });

  it("leaves out text that only names the files, by name, path, or file URL", () => {
    const files = [
      new File(["x"], "a report.pdf", { type: "application/pdf" }),
      new File(["x"], "b.txt", { type: "text/plain" }),
    ];
    for (const text of [
      "a report.pdf\nb.txt",
      "/home/someone/a report.pdf\r\n/home/someone/b.txt",
      "C:\\Users\\someone\\a report.pdf\nC:\\Users\\someone\\b.txt",
      "file:///home/someone/a%20report.pdf\nfile:///home/someone/b.txt\n",
    ]) {
      expect(transferredContent(carrying(files, text), named).text).toBe("");
    }
  });

  it("keeps text beside files that says more than their names", () => {
    const files = [new File(["x"], "image.png", { type: "image/png" })];
    expect(transferredContent(carrying(files, "Q3 figures"), named).text).toBe("Q3 figures");
    expect(transferredContent(carrying([], "image.png"), named).text).toBe("image.png");
  });
});

describe("timestampedName", () => {
  const at = new Date(2026, 10, 9, 8, 5, 7);

  it("writes the local moment in digits and the type's extension", () => {
    expect(timestampedName("img", "image/png", at)).toBe("img_20261109_080507.png");
    expect(timestampedName("img", "image/jpeg", at)).toBe("img_20261109_080507.jpg");
    expect(timestampedName("img", "image/svg+xml", at)).toBe("img_20261109_080507.svg");
  });

  it("writes no extension where the type gives none fit for one", () => {
    expect(timestampedName("file", "", at)).toBe("file_20261109_080507");
    expect(timestampedName("file", "application/vnd.ms-excel", at)).toBe("file_20261109_080507");
  });
});
