import type { RailFolder } from "@aspen/protocol";

/** Where a rail entry's community is: its deployment's domain, `null` for the user's home. */
export interface RailPlace {
  readonly domain: string | null;
  readonly communityId: string;
}

/** A community's key in the user's rail order: its id at home, `domain/id` elsewhere. */
export function railKey({ domain, communityId }: RailPlace): string {
  return domain === null ? communityId : `${domain}/${communityId}`;
}

const FOLDER_PREFIX = "folder:";

/** A folder's key in the rail order, which no community's key can be. */
export function folderKey(id: string): string {
  return FOLDER_PREFIX + id;
}

/** The folder a rail key names, or `null` when it names a community. */
export function folderIdOf(key: string): string | null {
  return key.startsWith(FOLDER_PREFIX) ? key.slice(FOLDER_PREFIX.length) : null;
}

/** How the user arranged the rail: its top-level order (`RAIL_ORDER`) and its folders. */
export interface RailLayout {
  readonly order: readonly string[];
  readonly folders: readonly RailFolder[];
}

/** One thing standing in the rail: a community, or a folder of them. */
export type RailUnit<T> =
  | { readonly kind: "community"; readonly entry: T }
  | { readonly kind: "folder"; readonly folder: RailFolder; readonly entries: readonly T[] };

/**
 * The rail in the user's arrangement: what `order` names as it names it, a folder standing
 * where its key does with its communities inside it, and then the rest as they come, which is
 * the home's communities in the home's order and then each other deployment's in its own, so a
 * community joined anywhere shows at the end. A community two folders claim is in the first; a
 * folder none of whose communities the user still belongs to is not shown.
 */
export function arrangeRail<T extends RailPlace>(
  entries: readonly T[],
  layout: RailLayout,
): RailUnit<T>[] {
  const byKey = new Map(entries.map((entry) => [railKey(entry), entry]));
  const placed = new Set<string>();
  const units: RailUnit<T>[] = [];
  const shownFolders = new Set<string>();
  // Each community is claimed by the first folder that names it.
  const members = new Map<string, T[]>();
  for (const folder of layout.folders) {
    const inside: T[] = [];
    for (const key of folder.members) {
      const entry = byKey.get(key);
      if (entry !== undefined && !placed.has(key)) {
        placed.add(key);
        inside.push(entry);
      }
    }
    members.set(folder.id, inside);
  }
  const addFolder = (folder: RailFolder) => {
    const inside = members.get(folder.id) ?? [];
    if (inside.length > 0 && !shownFolders.has(folder.id)) {
      shownFolders.add(folder.id);
      units.push({ kind: "folder", folder, entries: inside });
    }
  };
  for (const key of layout.order) {
    const id = folderIdOf(key);
    if (id !== null) {
      const folder = layout.folders.find((f) => f.id === id);
      if (folder !== undefined) {
        addFolder(folder);
      }
      continue;
    }
    const entry = byKey.get(key);
    if (entry !== undefined && !placed.has(key)) {
      placed.add(key);
      units.push({ kind: "community", entry });
    }
  }
  // A folder the order does not name stands after those it does.
  for (const folder of layout.folders) {
    addFolder(folder);
  }
  for (const entry of entries) {
    if (!placed.has(railKey(entry))) {
      units.push({ kind: "community", entry });
    }
  }
  return units;
}

/** Every community's key in rail order, folders opened in place. */
export function railSequence<T extends RailPlace>(units: readonly RailUnit<T>[]): string[] {
  return units.flatMap((unit) =>
    unit.kind === "community" ? [railKey(unit.entry)] : unit.entries.map(railKey),
  );
}

/** Where something is dropped in the rail: before, after, or on the row a key names. */
export interface RailDrop {
  readonly key: string;
  readonly position: "before" | "after" | "on";
}

type Unit =
  { kind: "community"; key: string } | { kind: "folder"; folder: RailFolder; members: string[] };

/**
 * The arrangement after dropping the row `dragged` at `drop`, or `null` when that drop means
 * nothing. The rail is one list in which an open folder's communities follow it:
 *
 * - a community dropped on another makes a folder of the two where the other stood, and one
 *   dropped on a folder or on one of its communities joins it;
 * - a community dropped before or after one of an open folder's communities goes into the folder
 *   there, except after its last, which takes it out to stand after the folder; just after an
 *   open folder's own row is its first place;
 * - a folder moves whole, and stands before or after the folder or community it is dropped by,
 *   its own place if that community is in a folder; it cannot go on anything;
 * - a folder left with one community goes, that community standing where it stood.
 *
 * `newId` names a folder the drop makes.
 */
export function moveInRail<T extends RailPlace>(
  units: readonly RailUnit<T>[],
  dragged: string,
  drop: RailDrop,
  newId: () => string,
): RailLayout | null {
  if (dragged === drop.key) {
    return null;
  }
  const model: Unit[] = units.map((unit) =>
    unit.kind === "community"
      ? { kind: "community", key: railKey(unit.entry) }
      : { kind: "folder", folder: unit.folder, members: unit.entries.map(railKey) },
  );
  const draggedFolder = folderIdOf(dragged);
  const topIndex = (key: string) =>
    model.findIndex((unit) =>
      unit.kind === "community" ? unit.key === key : folderKey(unit.folder.id) === key,
    );
  const containing = (key: string) =>
    model.find(
      (unit): unit is Extract<Unit, { kind: "folder" }> =>
        unit.kind === "folder" && unit.members.includes(key),
    );

  if (draggedFolder !== null) {
    if (drop.position === "on") {
      return null;
    }
    const from = topIndex(dragged);
    if (from < 0) {
      return null;
    }
    const [moving] = model.splice(from, 1);
    if (moving === undefined) {
      return null;
    }
    const host = containing(drop.key);
    const targetKey = host === undefined ? drop.key : folderKey(host.folder.id);
    // Dropped among a folder's own communities, it stands after that folder.
    const position = host === undefined ? drop.position : "after";
    const at = topIndex(targetKey);
    if (at < 0) {
      return null;
    }
    model.splice(position === "before" ? at : at + 1, 0, moving);
    return layoutOf(model);
  }

  // Take the community out of wherever it is.
  const source = containing(dragged);
  if (source !== undefined) {
    source.members.splice(source.members.indexOf(dragged), 1);
  } else {
    const from = topIndex(dragged);
    if (from < 0) {
      return null;
    }
    model.splice(from, 1);
  }

  const targetFolder = folderIdOf(drop.key);
  const host = containing(drop.key);
  if (targetFolder !== null) {
    const at = topIndex(drop.key);
    const unit = model[at];
    if (unit?.kind !== "folder") {
      return null;
    }
    if (drop.position === "on") {
      unit.members.push(dragged);
    } else if (drop.position === "after" && unit.folder.open) {
      unit.members.unshift(dragged);
    } else {
      model.splice(drop.position === "before" ? at : at + 1, 0, {
        kind: "community",
        key: dragged,
      });
    }
  } else if (host !== undefined) {
    const index = host.members.indexOf(drop.key);
    if (drop.position === "on") {
      host.members.splice(index + 1, 0, dragged);
    } else if (drop.position === "before") {
      host.members.splice(index, 0, dragged);
    } else if (index < host.members.length - 1) {
      host.members.splice(index + 1, 0, dragged);
    } else {
      const at = topIndex(folderKey(host.folder.id));
      model.splice(at + 1, 0, { kind: "community", key: dragged });
    }
  } else {
    const at = topIndex(drop.key);
    if (at < 0) {
      return null;
    }
    if (drop.position === "on") {
      model[at] = {
        kind: "folder",
        folder: { id: newId(), name: "", color: "accent", open: false, members: [] },
        members: [drop.key, dragged],
      };
    } else {
      model.splice(drop.position === "before" ? at : at + 1, 0, {
        kind: "community",
        key: dragged,
      });
    }
  }
  return layoutOf(dissolveSingles(model));
}

/** The arrangement with one folder changed by `change`. */
export function updateFolder<T extends RailPlace>(
  units: readonly RailUnit<T>[],
  id: string,
  change: Partial<Pick<RailFolder, "name" | "color" | "open">>,
): RailLayout {
  return layoutOf(
    units.map((unit): Unit =>
      unit.kind === "community"
        ? { kind: "community", key: railKey(unit.entry) }
        : {
            kind: "folder",
            folder: unit.folder.id === id ? { ...unit.folder, ...change } : unit.folder,
            members: unit.entries.map(railKey),
          },
    ),
  );
}

/** The arrangement with a folder undone, its communities standing where it stood. */
export function ungroupFolder<T extends RailPlace>(
  units: readonly RailUnit<T>[],
  id: string,
): RailLayout {
  return layoutOf(
    units.flatMap((unit): Unit[] => {
      if (unit.kind === "community") {
        return [{ kind: "community", key: railKey(unit.entry) }];
      }
      if (unit.folder.id === id) {
        return unit.entries.map((entry) => ({ kind: "community", key: railKey(entry) }));
      }
      return [{ kind: "folder", folder: unit.folder, members: unit.entries.map(railKey) }];
    }),
  );
}

/** Folders of one community or none give way to what they hold. */
function dissolveSingles(model: Unit[]): Unit[] {
  return model.flatMap((unit): Unit[] =>
    unit.kind === "folder" && unit.members.length < 2
      ? unit.members.map((key) => ({ kind: "community", key }))
      : [unit],
  );
}

function layoutOf(model: readonly Unit[]): RailLayout {
  return {
    order: model.map((unit) => (unit.kind === "community" ? unit.key : folderKey(unit.folder.id))),
    folders: model.flatMap((unit) =>
      unit.kind === "folder" ? [{ ...unit.folder, members: [...unit.members] }] : [],
    ),
  };
}
