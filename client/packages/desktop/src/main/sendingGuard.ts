import { type BrowserWindow, dialog, ipcMain } from "electron";

/**
 * Asking before the window closes while a message the reader sent is still uploading its
 * files: the page sends them, so closing it stops them and the message is never posted. The
 * page says when that is so (`sending:set`, from `packages/app/src/api/sendingGuard.ts`), with
 * the question in the reader's language, since the main process holds no catalogue; `null`
 * says nothing is on its way.
 */
interface Question {
  message: string;
  detail: string;
  stay: string;
  quit: string;
}

function questionOf(raw: unknown): Question | null {
  if (typeof raw !== "object" || raw === null) {
    return null;
  }
  const { message, detail, stay, quit } = raw as Record<string, unknown>;
  return typeof message === "string" &&
    typeof detail === "string" &&
    typeof stay === "string" &&
    typeof quit === "string"
    ? { message, detail, stay, quit }
    : null;
}

/** Holds `window` open, when the reader says so, while its page is still sending files. */
export function guardSending(window: BrowserWindow): void {
  let question: Question | null = null;
  const set = (event: Electron.IpcMainEvent, raw: unknown) => {
    if (event.sender === window.webContents) {
      question = questionOf(raw);
    }
  };
  ipcMain.on("sending:set", set);
  window.on("close", (event) => {
    if (question === null) {
      return;
    }
    const choice = dialog.showMessageBoxSync(window, {
      type: "question",
      message: question.message,
      detail: question.detail,
      buttons: [question.stay, question.quit],
      defaultId: 0,
      cancelId: 0,
    });
    if (choice === 0) {
      event.preventDefault();
    }
  });
  window.once("closed", () => {
    ipcMain.off("sending:set", set);
  });
}
