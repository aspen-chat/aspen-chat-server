/**
 * A picture's size in pixels as it shows, turned as its own orientation says, for readers to
 * make room for it before it loads; `undefined` for a file that is no picture, or one this
 * browser cannot read.
 */
export async function measurePicture(
  file: File,
): Promise<{ width: number; height: number } | undefined> {
  if (!file.type.startsWith("image/") || typeof createImageBitmap !== "function") {
    return undefined;
  }
  try {
    const bitmap = await createImageBitmap(file);
    try {
      return { width: bitmap.width, height: bitmap.height };
    } finally {
      bitmap.close();
    }
  } catch {
    return undefined;
  }
}
