import { beforeEach, describe, expect, it } from "vitest";
import { MAX_AGE_MS, MAX_DRAFTS, readDraft, writeDraft } from "./drafts";

const draft = (text: string) => ({ text, picks: [], attachments: [], echo: false });

describe("drafts", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("keeps a draft per account and channel, and forgets it once empty", () => {
    writeDraft("kate", "general", draft("hello"));
    expect(readDraft("kate", "general")?.text).toBe("hello");
    expect(readDraft("bob", "general")).toBeNull();
    expect(readDraft("kate", "dev")).toBeNull();
    writeDraft("kate", "general", draft("   "));
    expect(readDraft("kate", "general")).toBeNull();
  });

  it("keeps a draft that is only files", () => {
    const file = { id: "a" } as never;
    writeDraft("kate", "general", { ...draft(""), attachments: [file] });
    expect(readDraft("kate", "general")?.attachments).toHaveLength(1);
  });

  it("lets the oldest go past the limit, and any past the age", () => {
    for (let i = 0; i <= MAX_DRAFTS; i++) {
      writeDraft("kate", `c${String(i)}`, draft("x"), 1_000 + i);
    }
    expect(readDraft("kate", "c0")).toBeNull();
    expect(readDraft("kate", `c${String(MAX_DRAFTS)}`)).not.toBeNull();
    writeDraft("kate", "new", draft("y"), 1_000 + MAX_DRAFTS + MAX_AGE_MS + 1);
    expect(readDraft("kate", `c${String(MAX_DRAFTS)}`)).toBeNull();
    expect(readDraft("kate", "new")).not.toBeNull();
  });
});
