import { nativeCanChooseDestination, nativeChooseDestination } from "@/api/filesBridge";
import { safeFileName } from "@/features/voice/files";

/**
 * Saves `file` under `name`: in the mobile apps through the system's picker (`filesBridge`), whose
 * web view cannot follow a download link, and elsewhere as a download, which the browser or
 * Electron files where it files every download. Resolves once it is handed over; `false` when
 * the user backed out of the picker.
 */
export async function saveFile(file: Blob, name: string): Promise<boolean> {
  const safeName = safeFileName(name);
  if (nativeCanChooseDestination()) {
    const sink = await nativeChooseDestination(safeName);
    if (sink === null) {
      return false;
    }
    try {
      await sink.write(await file.arrayBuffer());
      await sink.close();
    } catch (e) {
      await sink.abort();
      throw e;
    }
    return true;
  }
  const url = URL.createObjectURL(file);
  const link = document.createElement("a");
  link.href = url;
  link.download = safeName;
  link.click();
  // The download reads the object after `click` returns, so it is released a while later.
  setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 60_000);
  return true;
}
