import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  ALL_PERMISSIONS,
  explain,
  resolveCommunity,
  type OverrideGrant,
  type Permission,
} from "../src";
import type { Role } from "../src/generated/events";

interface Case {
  name: string;
  roles: { id: string; position: number; permissions: Permission[]; everyone: boolean }[];
  owner: boolean;
  holds: string[];
  categoryOverrides: OverrideGrant[];
  channelOverrides: OverrideGrant[];
  community: Permission[];
  channel: Permission[];
  rank: number;
}

// The same cases the server's resolver runs, so the two agree.
const vectors = JSON.parse(
  readFileSync(new URL("../../../../spec/permission_vectors.json", import.meta.url), "utf8"),
) as { cases: Case[] };

/** A set as the server lists it: in the fixed order of every permission. */
function listed(set: ReadonlySet<Permission>): Permission[] {
  return ALL_PERMISSIONS.filter((p) => set.has(p));
}

describe("the shared permission vectors", () => {
  it("has cases", () => {
    expect(vectors.cases.length).toBeGreaterThan(0);
  });
  for (const c of vectors.cases) {
    it(c.name, () => {
      const roles: Role[] = c.roles.map((r) => ({ ...r, community: "c", name: r.id }));
      const access = resolveCommunity(roles, c.holds, c.owner);
      expect(listed(access.held)).toEqual(c.community);
      expect(listed(access.inChannel(c.categoryOverrides, c.channelOverrides))).toEqual(c.channel);
      expect(access.rank).toBe(c.rank);
      // The explanation reaches the same decision for every permission.
      const explained = ALL_PERMISSIONS.filter(
        (p) => explain(access, p, c.categoryOverrides, c.channelOverrides).allowed,
      );
      expect(explained).toEqual(c.channel);
    });
  }
});
