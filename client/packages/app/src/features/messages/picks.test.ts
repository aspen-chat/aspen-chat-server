import type { CustomEmoji } from "@aspen/protocol";
import { beforeEach, describe, expect, it } from "vitest";
import { quoteArgument, tokenize } from "@/features/commands/commandLine";
import { encodeCustomEmoji } from "@/features/emoji/customEmoji";
import { readDraft } from "@/features/messages/drafts";
import {
  decodePicked,
  encodePicked,
  pickAt,
  retype,
  soundPicks,
  splice,
  trimPicked,
  type PickedText,
  type PickKind,
  type PickNames,
  type PickTarget,
} from "@/features/messages/picks";

const A = "01a0e95a-8b0f-75df-a00b-29a4f0b878d1";
const B = "01a0e95a-8b0f-75df-a00b-29a4f0b878d2";
const E1 = "0190f0a0-0000-7000-8000-0000000000e1";
const E2 = "0190f0a0-0000-7000-8000-0000000000e2";

const user = (id: string) => ({ kind: "user", id }) as const;
const role = (id: string) => ({ kind: "role", id }) as const;
const emoji = (id: string) => ({ kind: "emoji", id }) as const;

/** Writes each part at the end, as picking from a list or typing does. */
function write(...parts: (string | [string, PickTarget])[]) {
  let held: PickedText = { text: "", picks: [] };
  for (const part of parts) {
    const at = held.text.length;
    held =
      typeof part === "string" ? retype(held, held.text + part) : splice(held, at, at, ...part);
  }
  return held;
}

describe("picks", () => {
  it("sends two picks that read the same as their own targets", () => {
    const held = write(["@alice", user(A)], " ", ["@alice", user(B)], " ", ["@alice", role(A)]);
    expect(held.text).toBe("@alice @alice @alice");
    expect(encodePicked(held)).toBe(`<@${A}> <@${B}> <@&${A}>`);
  });

  it("leaves a pick's name alone where it was typed by hand", () => {
    const held = write(
      ["@alice", user(A)],
      " write to bob@alice.dev or `@alice` and @alice\n```\n@alice\n```",
    );
    expect(encodePicked(held)).toBe(
      `<@${A}> write to bob@alice.dev or \`@alice\` and @alice\n\`\`\`\n@alice\n\`\`\``,
    );
  });

  it("carries a pick past an edit before it and one after it", () => {
    const held = write("hi ", ["@alice", user(A)], " there");
    const before = retype(held, "oh hi @alice there");
    expect(before.picks).toEqual([{ ...user(A), start: 6, end: 12 }]);
    expect(encodePicked(before)).toBe(`oh hi <@${A}> there`);
    const after = retype(before, "oh hi @alice here");
    expect(after.picks).toEqual(before.picks);
    const gone = retype(after, "@alice here");
    expect(encodePicked(gone)).toBe(`<@${A}> here`);
  });

  it("makes a pick plain text once its own text is edited", () => {
    const held = write(["@alice", user(A)], " and ", ["@bob", user(B)]);
    for (const next of ["@alce and @bob", "@alicia and @bob", "@ali ce and @bob", " and @bob"]) {
      const edited = retype(held, next);
      expect(edited.picks.map((pick) => pick.id)).toEqual([B]);
      expect(encodePicked(edited)).toBe(next.replace("@bob", `<@${B}>`));
    }
    // Replacing a selection that reaches into two picks ends both.
    expect(retype(held, "@alib").picks).toEqual([]);
  });

  it("keeps a pick that text is written beside", () => {
    const held = write(["@kate", user(A)]);
    expect(encodePicked(retype(held, "@kateさん"))).toBe(`<@${A}>さん`);
    expect(encodePicked(retype(held, "(@kate"))).toBe(`(<@${A}>`);
  });

  it("places an edit by the caret where the texts alone do not say", () => {
    const held = write(["@anna", user(A)]);
    // An `a` typed after `@ann`, inside the pick, and one typed after the whole of it.
    expect(retype(held, "@annaa", 5).picks).toEqual([]);
    expect(retype(held, "@annaa", 6).picks).toHaveLength(1);
    // A colon typed before a pick that begins with one.
    const blob = write([":blob:", emoji(E1)]);
    expect(retype(blob, "::blob:", 1).picks).toEqual([{ ...emoji(E1), start: 1, end: 7 }]);
    // With no caret to say, the placing that keeps the pick is taken.
    expect(retype(blob, "::blob:").picks).toEqual([{ ...emoji(E1), start: 1, end: 7 }]);
    // A caret the text after which is not how the text ended is not believed.
    expect(retype(held, "x @anna", 1).picks).toEqual([{ ...user(A), start: 2, end: 7 }]);
    // Text pasted over the whole of it ends its picks, though it ends as they did.
    expect(retype(held, "x @anna", 7).picks).toEqual([]);
  });

  it("writes a completion over what was typed, moving the picks after it", () => {
    const held = write("@al and ", ["@bob", user(B)]);
    const picked = splice(held, 0, 3, "@alice", user(A), " ");
    expect(picked.text).toBe("@alice  and @bob");
    expect(encodePicked(picked)).toBe(`<@${A}>  and <@${B}>`);
  });

  it("sends a picked custom emoji by its id, and one typed by hand by its name", () => {
    const upper: CustomEmoji = { id: E1, community: "c", name: "Éx", icon: "i", createdBy: null };
    const lower: CustomEmoji = { ...upper, id: E2, name: "éx" };
    const named = (text: string) => encodeCustomEmoji(text, [upper, lower]);
    // Picked, each is its own whichever name the other has; typed, the name as written decides.
    const held = write([":ÉX:", emoji(E2)], " ", [":éx:", emoji(E1)], " :éx: :ÉX:");
    expect(encodePicked(held, named)).toBe(`<:${E2}> <:${E1}> <:${E2}> :ÉX:`);
  });

  it("leaves a command's picks as written in a message", () => {
    const held = write(["#general", { kind: "channel", id: A }], " ", ["@alice", user(B)]);
    expect(encodePicked(held)).toBe(`#general <@${B}>`);
  });

  it("finds each word of a command line that was picked, by its place", () => {
    const channel = (id: string) => ({ kind: "channel", id }) as const;
    const held = write(
      "/move ",
      ["#general", channel(A)],
      " ",
      ["#general", channel(B)],
      " ",
      [quoteArgument("@Team leads"), role(A)],
      " #general",
    );
    const words = tokenize(held.text, 5);
    const picked = (index: number, kind: PickKind) => {
      const word = words[index];
      return word === undefined ? undefined : pickAt(held.picks, word.start, word.end, kind)?.id;
    };
    expect([picked(0, "channel"), picked(1, "channel"), picked(2, "role")]).toEqual([A, B, A]);
    // A word typed by hand is no pick, nor is a pick of another kind than the parameter takes.
    expect(picked(3, "channel")).toBeUndefined();
    expect(picked(0, "role")).toBeUndefined();
    // A word that has grown past its pick is no longer the pick.
    const grown = retype(held, held.text.replace("#general ", "#general-2 "));
    const first = tokenize(grown.text, 5)[0];
    expect(pickAt(grown.picks, first?.start ?? -1, first?.end ?? -1, "channel")).toBeUndefined();
  });

  it("trims the text as it is sent, its picks moved with it", () => {
    const held = write("  ", ["@alice", user(A)], " hi \n");
    const trimmed = trimPicked(held);
    expect(trimmed).toEqual({ text: "@alice hi", picks: [{ ...user(A), start: 0, end: 6 }] });
  });

  const names: PickNames = {
    usernameOf: (id) => (id === A || id === B ? "alice" : undefined),
    roleNameOf: (id) => (id === A ? "alice" : undefined),
    emojiNameOf: (id) => (id === E1 ? "blob" : undefined),
  };

  it("reads a sent message back as it was written, and sends it back the same", () => {
    const sent = `<@${A}> <@${B}> <@&${A}> <:${E1}> @alice :blob: <@&${B}> <:${E2}> \`<@${A}>\``;
    const decoded = decodePicked(sent, names);
    expect(decoded.text).toBe(
      `@alice @alice @alice :blob: @alice :blob: <@&${B}> <:${E2}> \`@alice\``,
    );
    expect(decoded.picks.map((pick) => `${pick.kind}:${pick.id}`)).toEqual([
      `user:${A}`,
      `user:${B}`,
      `role:${A}`,
      `emoji:${E1}`,
      `user:${A}`,
    ]);
    expect(encodePicked(trimPicked(decoded))).toBe(sent);
    // An edit elsewhere leaves every tag whose text it did not touch.
    const edited = retype(decoded, decoded.text.replace(" :blob: <", " hello <"));
    expect(encodePicked(edited)).toBe(sent.replace(" :blob: <", " hello <"));
  });

  it("keeps the case an id was sent in", () => {
    const sent = `<@${A.toUpperCase()}>`;
    const loose: PickNames = { ...names, usernameOf: () => "alice" };
    expect(encodePicked(decodePicked(sent, loose))).toBe(sent);
  });

  describe("in a kept draft", () => {
    beforeEach(() => {
      window.localStorage.clear();
    });

    const keep = (channel: string, draft: unknown) => {
      window.localStorage.setItem("aspen.drafts", JSON.stringify({ [`kate:${channel}`]: draft }));
    };

    it("are restored with the text, and send as they would have", () => {
      const held = write(["@alice", user(A)], " and ", ["@alice", user(B)], " or bob@alice.dev");
      keep("restored", { ...held, attachments: [], echo: false, savedAt: 1 });
      const restored = readDraft("kate", "restored");
      expect(restored?.picks).toEqual(held.picks);
      expect(encodePicked({ text: restored?.text ?? "", picks: restored?.picks ?? [] })).toBe(
        `<@${A}> and <@${B}> or bob@alice.dev`,
      );
    });

    it("are left out where storage holds something else", () => {
      keep("other-shape", {
        text: "@alice hi",
        picks: [{ text: "@alice", token: `<@${A}>` }],
        attachments: [],
        echo: false,
        savedAt: 1,
      });
      expect(readDraft("kate", "other-shape")?.picks).toEqual([]);
      keep("no-picks", { text: "hi", attachments: [], echo: false, savedAt: 1 });
      expect(readDraft("kate", "no-picks")?.picks).toEqual([]);
    });
  });

  it("takes from storage only picks that are whole, in the text, and apart", () => {
    const sound = soundPicks("@alice @bob", [
      { ...user(B), start: 7, end: 11 },
      { ...user(A), start: 0, end: 6 },
      { ...user(A), start: 4, end: 8 },
      { ...user(A), start: 9, end: 40 },
      { ...user(A), start: 3, end: 3 },
      { kind: "planet", id: A, start: 0, end: 1 },
      { kind: "user", start: 0, end: 1 },
      null,
      "pick",
    ]);
    expect(sound).toEqual([
      { ...user(A), start: 0, end: 6 },
      { ...user(B), start: 7, end: 11 },
    ]);
    expect(soundPicks("x", undefined)).toEqual([]);
  });
});
