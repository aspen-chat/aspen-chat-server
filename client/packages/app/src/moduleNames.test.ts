import { readdirSync, statSync } from "node:fs";
import { dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * Modules are imported without their extension, and macOS and Windows find files without
 * regard to case, so two modules in one folder whose names differ only in case (or extension
 * and case) resolve to the same file there while Linux tells them apart.
 */
describe("module names", () => {
  it("never differ only in case within a folder", () => {
    const root = dirname(fileURLToPath(import.meta.url));
    const clashes: string[] = [];
    const walk = (folder: string) => {
      const seen = new Map<string, string>();
      for (const entry of readdirSync(folder)) {
        const path = join(folder, entry);
        if (statSync(path).isDirectory()) {
          walk(path);
          continue;
        }
        const module = entry.slice(0, entry.length - extname(entry).length).toLowerCase();
        const other = seen.get(module);
        if (other !== undefined && other !== entry) {
          clashes.push(`${join(folder, other)} and ${entry}`);
        }
        seen.set(module, entry);
      }
    };
    walk(root);
    walk(join(root, "..", "e2e"));
    expect(clashes).toEqual([]);
  });
});
