import { FontFileError, readFontInfo, type FontFileProblem, type FontInfo } from "./fontFile";

/**
 * The font files the user has added on this install, kept whole in IndexedDB (a family of a
 * script with many characters runs to tens of megabytes, far past what `localStorage` holds) and
 * never sent anywhere. Each file is a face, grouped with the others of its family by the name
 * it gives itself; adding a face of a family, weight, and style already kept replaces it.
 * Families are registered with `document.fonts` under an alias of their own when first used, so
 * a name the user's fonts share with one the system has installed cannot pick the system's.
 * What each face says of itself is kept apart from its file, so listing the library reads no
 * font data.
 */

export interface StoredFace extends FontInfo {
  id: string;
  fileName: string;
}

export interface FontFamily {
  name: string;
  faces: readonly StoredFace[];
}

export interface RefusedFont {
  fileName: string;
  problem: FontFileProblem | "unreadable";
}

export type AddedFont = { fileName: string; family: string } | RefusedFont;

const DATABASE = "aspen-fonts";
/** `StoredFace`s, by id. */
const FACES = "faces";
/** Each face's file, an `ArrayBuffer`, under the face's id. */
const FILES = "files";

let opened: Promise<IDBDatabase> | undefined;

function database(): Promise<IDBDatabase> {
  opened ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(FACES, { keyPath: "id" });
      request.result.createObjectStore(FILES);
    };
    request.onsuccess = () => {
      resolve(request.result);
    };
    request.onerror = () => {
      reject(request.error ?? new Error("the font library could not be opened"));
    };
  }).catch((error: unknown) => {
    opened = undefined;
    throw error;
  });
  return opened;
}

function settled<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => {
      resolve(request.result);
    };
    request.onerror = () => {
      reject(request.error ?? new Error("the font library could not be read"));
    };
  });
}

function committed(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => {
      resolve();
    };
    transaction.onerror = transaction.onabort = () => {
      reject(transaction.error ?? new Error("the font library could not be written"));
    };
  });
}

async function allFaces(): Promise<StoredFace[]> {
  const db = await database();
  return settled(db.transaction(FACES).objectStore(FACES).getAll() as IDBRequest<StoredFace[]>);
}

/** Deletes each face and its file in `transaction`. */
function deleteFaces(transaction: IDBTransaction, faces: readonly StoredFace[]): void {
  for (const face of faces) {
    transaction.objectStore(FACES).delete(face.id);
    transaction.objectStore(FILES).delete(face.id);
  }
}

async function fileOf(id: string): Promise<ArrayBuffer | undefined> {
  const db = await database();
  return settled(
    db.transaction(FILES).objectStore(FILES).get(id) as IDBRequest<ArrayBuffer | undefined>,
  );
}

function sameFace(a: FontInfo, b: FontInfo): boolean {
  return a.family === b.family && a.style === b.style && String(a.weight) === String(b.weight);
}

/** Every family kept, by name, each face lightest first and upright before slanted. */
export async function listFamilies(): Promise<FontFamily[]> {
  const families = new Map<string, StoredFace[]>();
  for (const face of await allFaces()) {
    const faces = families.get(face.family) ?? [];
    faces.push(face);
    families.set(face.family, faces);
  }
  const lightest = (face: StoredFace) =>
    typeof face.weight === "number" ? face.weight : face.weight[0];
  return [...families]
    .map(([name, faces]) => ({
      name,
      faces: faces.sort(
        (a, b) =>
          lightest(a) - lightest(b) || Number(a.style !== "normal") - Number(b.style !== "normal"),
      ),
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** Reads and keeps each of `files`, saying for each which family it joined or why it did not. */
export async function addFontFiles(files: readonly File[]): Promise<AddedFont[]> {
  const read = await Promise.all(
    files.map(
      async (file): Promise<{ added: AddedFont; face?: StoredFace; data?: ArrayBuffer }> => {
        let data: ArrayBuffer;
        try {
          data = await file.arrayBuffer();
        } catch {
          return { added: { fileName: file.name, problem: "unreadable" } };
        }
        try {
          const info = await readFontInfo(data);
          return {
            added: { fileName: file.name, family: info.family },
            face: { ...info, id: crypto.randomUUID(), fileName: file.name },
            data,
          };
        } catch (error) {
          const problem = error instanceof FontFileError ? error.problem : "unrecognized";
          return { added: { fileName: file.name, problem } };
        }
      },
    ),
  );
  const adding = read.flatMap(({ face, data }) =>
    face === undefined || data === undefined ? [] : [{ face, data }],
  );
  if (adding.length > 0) {
    const existing = await allFaces();
    const db = await database();
    const transaction = db.transaction([FACES, FILES], "readwrite");
    const kept: StoredFace[] = [];
    for (const { face, data } of adding) {
      deleteFaces(
        transaction,
        [...existing, ...kept].filter((old) => sameFace(old, face)),
      );
      transaction.objectStore(FACES).put(face);
      transaction.objectStore(FILES).put(data, face.id);
      kept.push(face);
    }
    await committed(transaction);
    for (const family of new Set(kept.map((face) => face.family))) {
      forget(family);
    }
    // Asks the browser not to clear the library under storage pressure; it may decline.
    if (window.isSecureContext) {
      void navigator.storage.persist().catch(() => undefined);
    }
  }
  return read.map(({ added }) => added);
}

/** Forgets every face of `family`. */
export async function removeFamily(family: string): Promise<void> {
  const doomed = (await allFaces()).filter((face) => face.family === family);
  const db = await database();
  const transaction = db.transaction([FACES, FILES], "readwrite");
  deleteFaces(transaction, doomed);
  await committed(transaction);
  forget(family);
}

/** The weight and style a face says it has, as a `FontFace` takes them. */
function descriptors(face: StoredFace): { weight: string; style: string } {
  return {
    weight: typeof face.weight === "number" ? String(face.weight) : face.weight.join(" "),
    style: face.style,
  };
}

interface Registered {
  alias: string;
  faces: Promise<FontFace[]>;
}

const registered = new Map<string, Registered>();
let aliases = 0;

function forget(family: string): void {
  const entry = registered.get(family);
  if (entry === undefined) {
    return;
  }
  registered.delete(family);
  void entry.faces.then((faces) => {
    for (const face of faces) {
      document.fonts.delete(face);
    }
  });
}

/**
 * The alias under which `family` is drawn, after registering its faces with the document, or
 * `undefined` when no face of it is kept. Faces the browser cannot draw are left out.
 */
export async function loadFamily(family: string): Promise<string | undefined> {
  let entry = registered.get(family);
  if (entry === undefined) {
    const alias = `AspenFont${String(++aliases)}`;
    const faces = allFaces().then((stored) =>
      Promise.all(
        stored
          .filter((face) => face.family === family)
          .map(async (stored) => {
            const data = await fileOf(stored.id);
            if (data === undefined) {
              return undefined;
            }
            const face = new FontFace(alias, data, descriptors(stored));
            try {
              await face.load();
            } catch {
              return undefined;
            }
            document.fonts.add(face);
            return face;
          }),
      ).then((loaded) => loaded.filter((face) => face !== undefined)),
    );
    entry = { alias, faces };
    registered.set(family, entry);
    faces.catch(() => {
      registered.delete(family);
    });
  }
  const faces = await entry.faces;
  return faces.length > 0 ? entry.alias : undefined;
}

/** A face of the user's, as a page that cannot read the library registers it. */
export interface FaceFile {
  /** The alias its family is drawn under. */
  family: string;
  weight: string;
  style: string;
  data: ArrayBuffer;
}

/**
 * The files of the families registered under `aliases`, each named by its alias, for a page
 * that cannot read the library and registers them itself: a plugin's view, which is in an origin
 * of its own.
 */
export async function facesUnder(aliases: readonly string[]): Promise<FaceFile[]> {
  const families = new Map(
    [...registered]
      .filter(([, entry]) => aliases.includes(entry.alias))
      .map(([family, entry]) => [family, entry.alias]),
  );
  if (families.size === 0) {
    return [];
  }
  const files = await Promise.all(
    (await allFaces()).flatMap((stored) => {
      const alias = families.get(stored.family);
      return alias === undefined
        ? []
        : [
            fileOf(stored.id).then((data) =>
              data === undefined ? undefined : { family: alias, ...descriptors(stored), data },
            ),
          ];
    }),
  );
  return files.filter((file) => file !== undefined);
}
