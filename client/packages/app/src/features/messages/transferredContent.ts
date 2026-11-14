/**
 * What a paste or a drop brings to a message: the files it carries, and its text. Text that only
 * names the files beside it (a file manager puts a copied file's name or path on the clipboard
 * with the file) is left out, since the files say it already.
 */
export interface TransferredContent {
  files: File[];
  text: string;
}

/** The name browsers give image data that came from no file (a screenshot, "Copy image"). */
const UNNAMED_IMAGE = /^image\.[a-z0-9]+$/i;

/** The extensions written on files of these types when they are named here. */
const EXTENSIONS: Readonly<Record<string, string>> = {
  "image/jpeg": "jpg",
  "image/svg+xml": "svg",
  "image/x-icon": "ico",
  "image/vnd.microsoft.icon": "ico",
  "text/plain": "txt",
};

/**
 * Reads `data` as content for a message, naming each file that came without a name of its own
 * as `name` says.
 */
export function transferredContent(
  data: DataTransfer,
  name: (file: File) => string,
): TransferredContent {
  const carried =
    data.files.length > 0
      ? Array.from(data.files)
      : Array.from(data.items).flatMap((item) => {
          const file = item.kind === "file" ? item.getAsFile() : null;
          return file === null ? [] : [file];
        });
  const files = carried.map((file) =>
    file.name === "" || (file.type.startsWith("image/") && UNNAMED_IMAGE.test(file.name))
      ? new File([file], name(file), { type: file.type, lastModified: file.lastModified })
      : file,
  );
  const text = data.getData("text/plain");
  return { files, text: namesOnly(text, carried) ? "" : text };
}

/** Whether every line of `text` names one of `files`, by its name, its path, or a file URL. */
function namesOnly(text: string, files: readonly File[]): boolean {
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "");
  if (lines.length === 0 || files.length === 0) {
    return false;
  }
  const names = new Set(files.map((file) => file.name));
  return lines.every((line) => names.has(lastSegment(line)));
}

function lastSegment(path: string): string {
  const segment = path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1);
  if (!path.startsWith("file:")) {
    return segment;
  }
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

/**
 * The name for a file of `mimeType` that came with none: `stem`, the moment it was named, and
 * the type's extension, as `img_20261110_140327.png`. The moment is local time, in digits that
 * sort as it does.
 */
export function timestampedName(stem: string, mimeType: string, at: Date): string {
  const two = (n: number) => String(n).padStart(2, "0");
  const date = `${String(at.getFullYear())}${two(at.getMonth() + 1)}${two(at.getDate())}`;
  const time = `${two(at.getHours())}${two(at.getMinutes())}${two(at.getSeconds())}`;
  const extension = extensionOf(mimeType);
  return `${stem}_${date}_${time}${extension === "" ? "" : `.${extension}`}`;
}

/** The extension for `mimeType`: its own where it has one, else its subtype when that is plain. */
function extensionOf(mimeType: string): string {
  const type = mimeType.split(";")[0]?.trim().toLowerCase() ?? "";
  const known = EXTENSIONS[type];
  if (known !== undefined) {
    return known;
  }
  const subtype = type.split("/")[1] ?? "";
  return /^[a-z0-9]+$/.test(subtype) ? subtype : "";
}
