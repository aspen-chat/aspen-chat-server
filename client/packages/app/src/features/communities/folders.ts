import type { FolderColor, RailFolder } from "@aspen/protocol";
import type { Messages } from "@/i18n/messages";

/** Each folder colour's background, written out whole so the stylesheet has every one. */
export const FOLDER_TINT: Record<FolderColor, string> = {
  accent: "bg-folder-accent",
  sky: "bg-folder-sky",
  violet: "bg-folder-violet",
  rose: "bg-folder-rose",
  amber: "bg-folder-amber",
  slate: "bg-folder-slate",
};

/** What a folder is called on screen: its name, or a plain word until it has one. */
export function folderName(m: Messages, folder: RailFolder): string {
  return folder.name.trim() === "" ? m.folders.untitled : folder.name;
}
