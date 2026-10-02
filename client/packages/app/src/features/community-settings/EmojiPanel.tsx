import { ApiProblemError } from "@aspen/protocol";
import { useRef, useState } from "react";
import { Button, Input, Label, TextField } from "react-aria-components";
import { useAccess, useCustomEmoji, useSync } from "@/api/hooks";
import { alertClass, fieldClass, hintClass, inputClass, labelClass } from "@/features/auth/styles";
import { CustomEmojiGlyph } from "@/features/emoji/CustomEmojiGlyph";
import {
  EMOJI_MAX_PX,
  EMOJI_TYPES,
  prepareEmojiPicture,
  type PictureProblem,
} from "@/features/emoji/emojiPicture";
import {
  dangerButtonClass,
  planeClass,
  planeSurfaceClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

function problemText(e: unknown): string {
  return e instanceof ApiProblemError ? e.message : String(e);
}

/**
 * The community's own emoji, by name, each with its picture; holders of Manage custom emoji
 * add one from a picture and a name, rename one in place, and remove one, which takes its
 * reactions with it.
 */
export function EmojiPanel({ communityId }: { communityId: string }) {
  const m = useMessages();
  const access = useAccess(communityId);
  const manage = access?.has("manageCustomEmoji") ?? false;
  const emoji = useCustomEmoji(communityId);
  return (
    <div className="flex flex-col gap-3">
      {manage && <AddEmoji communityId={communityId} />}
      <section className={planeClass} aria-labelledby="emoji-list-heading">
        <h3 id="emoji-list-heading" className="font-medium">
          {format(m.emojiPanel.listHeading, { count: String(emoji.length) })}
        </h3>
        {emoji.length === 0 ? (
          <p className={hintClass}>{manage ? m.emojiPanel.emptyManage : m.emojiPanel.empty}</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {emoji.map((e) => (
              <EmojiRow
                key={e.id}
                id={e.id}
                name={e.name}
                communityId={communityId}
                manage={manage}
              />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/** The form that adds an emoji: a picture, scaled down where it can be, and a name. */
function AddEmoji({ communityId }: { communityId: string }) {
  const m = useMessages();
  const sync = useSync();
  const fileInput = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");
  const [picture, setPicture] = useState<{ blob: Blob; mimeType: string; url: string } | null>(
    null,
  );
  const [problem, setProblem] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  async function choose(file: File | undefined) {
    setProblem(null);
    if (file === undefined) {
      return;
    }
    const prepared = await prepareEmojiPicture(file);
    if ("problem" in prepared) {
      setProblem(pictureProblem(prepared.problem));
      return;
    }
    if (picture !== null) {
      URL.revokeObjectURL(picture.url);
    }
    setPicture({ ...prepared, url: URL.createObjectURL(prepared.blob) });
    if (name === "") {
      // The file's name, less its extension, is a fair first guess at the emoji's.
      setName(
        file.name
          .replace(/\.[^.]*$/, "")
          .replace(/[\s:]+/g, "_")
          .slice(0, 32),
      );
    }
  }

  function pictureProblem(which: PictureProblem): string {
    switch (which) {
      case "type":
        return m.emojiPanel.pictureType;
      case "gifTooLarge":
        return format(m.emojiPanel.gifTooLarge, { px: String(EMOJI_MAX_PX) });
      case "size":
        return m.emojiPanel.pictureSize;
    }
  }

  async function add() {
    if (picture === null || adding) {
      return;
    }
    setAdding(true);
    setProblem(null);
    try {
      await sync.createCustomEmoji(communityId, name.trim(), picture.blob, picture.mimeType);
      URL.revokeObjectURL(picture.url);
      setPicture(null);
      setName("");
      if (fileInput.current !== null) {
        fileInput.current.value = "";
      }
    } catch (e) {
      setProblem(problemText(e));
    } finally {
      setAdding(false);
    }
  }

  return (
    <section className={planeClass} aria-labelledby="emoji-add-heading">
      <h3 id="emoji-add-heading" className="font-medium">
        {m.emojiPanel.addHeading}
      </h3>
      <p className={hintClass}>{format(m.emojiPanel.addHint, { px: String(EMOJI_MAX_PX) })}</p>
      <div className="flex flex-wrap items-end gap-3">
        <div className="flex items-center gap-2">
          <span
            aria-hidden="true"
            className={
              "flex h-12 w-12 items-center justify-center rounded-md " +
              (picture === null ? "border border-dashed border-line" : "")
            }
          >
            {picture !== null && (
              <img src={picture.url} alt="" className="max-h-12 max-w-12 object-contain" />
            )}
          </span>
          <input
            ref={fileInput}
            type="file"
            accept={EMOJI_TYPES.join(",")}
            className="sr-only"
            aria-label={m.emojiPanel.pictureLabel}
            onChange={(event) => {
              void choose(event.currentTarget.files?.[0]);
            }}
          />
          <Button
            className={secondaryButtonClass}
            onPress={() => {
              fileInput.current?.click();
            }}
          >
            {picture === null ? m.emojiPanel.choosePicture : m.emojiPanel.changePicture}
          </Button>
        </div>
        <TextField
          value={name}
          onChange={setName}
          maxLength={32}
          className={fieldClass + " w-56"}
          isRequired
        >
          <Label className={labelClass}>{m.emojiPanel.nameLabel}</Label>
          <Input className={inputClass} />
        </TextField>
        <Button
          className={secondaryButtonClass}
          isDisabled={picture === null || name.trim().length < 2 || adding}
          onPress={() => {
            void add();
          }}
        >
          {adding ? m.emojiPanel.adding : m.emojiPanel.add}
        </Button>
      </div>
      {problem !== null && (
        <p role="alert" className={alertClass}>
          {problem}
        </p>
      )}
    </section>
  );
}

/** One emoji: its picture and name, and, for those who may, renaming and removing it. */
function EmojiRow({
  id,
  name,
  communityId,
  manage,
}: {
  id: string;
  name: string;
  communityId: string;
  manage: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const [renaming, setRenaming] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  async function rename() {
    if (renaming === null || busy) {
      return;
    }
    const next = renaming.trim();
    if (next === name) {
      setRenaming(null);
      return;
    }
    setBusy(true);
    setProblem(null);
    try {
      await sync.renameCustomEmoji(id, next);
      setRenaming(null);
    } catch (e) {
      setProblem(problemText(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (busy) {
      return;
    }
    setBusy(true);
    setProblem(null);
    try {
      await sync.deleteCustomEmoji(id);
    } catch (e) {
      setProblem(problemText(e));
      setBusy(false);
      setRemoving(false);
    }
  }

  return (
    <li className={planeSurfaceClass + " flex flex-wrap items-center gap-3 px-3 py-2"}>
      <CustomEmojiGlyph id={id} communityId={communityId} size="large" />
      {renaming === null ? (
        <span className="min-w-0 flex-1 truncate">:{name}:</span>
      ) : (
        <TextField
          value={renaming}
          onChange={setRenaming}
          maxLength={32}
          aria-label={m.emojiPanel.nameLabel}
          autoFocus
          className="min-w-0 flex-1"
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void rename();
            } else if (event.key === "Escape") {
              event.preventDefault();
              setRenaming(null);
            }
          }}
        >
          <Input className={inputClass} />
        </TextField>
      )}
      {manage && !removing && renaming === null && (
        <>
          <Button
            className={secondaryButtonClass}
            onPress={() => {
              setRenaming(name);
            }}
          >
            {m.emojiPanel.rename}
          </Button>
          <Button
            className={dangerButtonClass}
            onPress={() => {
              setRemoving(true);
            }}
          >
            {m.emojiPanel.remove}
          </Button>
        </>
      )}
      {manage && renaming !== null && (
        <>
          <Button
            className={secondaryButtonClass}
            isDisabled={busy || renaming.trim().length < 2}
            onPress={() => {
              void rename();
            }}
          >
            {m.emojiPanel.save}
          </Button>
          <Button
            className={secondaryButtonClass}
            onPress={() => {
              setRenaming(null);
            }}
          >
            {m.emojiPanel.cancel}
          </Button>
        </>
      )}
      {manage && removing && (
        <span className="flex items-center gap-2 text-sm">
          <span>{m.emojiPanel.removeConfirm}</span>
          <Button
            className={dangerButtonClass}
            isDisabled={busy}
            onPress={() => {
              void remove();
            }}
          >
            {m.emojiPanel.removeNow}
          </Button>
          <Button
            className={secondaryButtonClass}
            onPress={() => {
              setRemoving(false);
            }}
          >
            {m.emojiPanel.cancel}
          </Button>
        </span>
      )}
      {problem !== null && (
        <p role="alert" className={alertClass + " basis-full"}>
          {problem}
        </p>
      )}
    </li>
  );
}
