import { me, minutesAgo } from "./fixtures";
import { reply } from "./reply";

/** Another deployment of the world's, as the admin API lists it. */
interface WorldDeployment {
  domain: string;
  origin: "administrator" | "terminal" | "firstContact";
  addedBy: string | null;
  createdAt: string;
  note: string | null;
  publicKey: string | null;
  publicKeyFingerprint: string | null;
  firstContactAt: string | null;
  lastContactAt: string | null;
  offeredKey: string | null;
  offeredKeyFingerprint: string | null;
  offeredKeyAt: string | null;
  lists: string[];
  protocol: { version: number; minimum: number; capabilities: string[] } | null;
  software: { name: string; version: string } | null;
  compatible: boolean;
  admission: {
    usersEmigration: boolean;
    usersImmigration: boolean;
    botsEmigration: boolean;
    botsImmigration: boolean;
  };
}

/** The deployment the world knows whose key has changed since it was pinned. */
export const rekeyedDeployment = "chat.example.org";
/** The deployment the world knows that answers with its pinned key. */
export const friendlyDeployment = "friends.example.net:8443";

/**
 * The world's federation: this deployment lets its users go only where it allows and takes
 * anyone's, and keeps its bots home. It knows two deployments; the first has offered a new key.
 */
export function federationWorld() {
  const admission = (lists: string[]) => ({
    usersEmigration: lists.includes("usersEmigrationAllow"),
    usersImmigration: true,
    botsEmigration: false,
    botsImmigration: false,
  });
  const known = (domain: string, extra: Partial<WorldDeployment>): WorldDeployment => ({
    domain,
    origin: "administrator",
    addedBy: me,
    createdAt: minutesAgo(3000),
    note: null,
    publicKey: "cGlubmVk",
    publicKeyFingerprint: `SHA256:pinned-${domain}`,
    firstContactAt: minutesAgo(3000),
    lastContactAt: minutesAgo(60),
    offeredKey: null,
    offeredKeyFingerprint: null,
    offeredKeyAt: null,
    lists: [],
    admission: admission([]),
    protocol: { version: 1, minimum: 1, capabilities: [] },
    software: { name: "aspen", version: "0.1.0" },
    compatible: true,
    ...extra,
  });
  const deployments: WorldDeployment[] = [
    known(rekeyedDeployment, {
      note: "the neighbours",
      offeredKey: "bmV3",
      offeredKeyFingerprint: "SHA256:offered-key",
      offeredKeyAt: minutesAgo(5),
    }),
    known(friendlyDeployment, { origin: "firstContact", addedBy: null }),
  ];
  const find = (domain: string) => deployments.find((d) => d.domain === domain);
  // The gates, as `PATCH /admin/federation` names them.
  const gates: Record<string, string | boolean> = {
    usersEmigration: "allowList",
    usersImmigration: "open",
    usersSharedList: false,
    usersImmigrationInviteRequired: false,
    botsEmigration: "closed",
    botsImmigration: "closed",
    botsSharedList: false,
    botsImmigrationInviteRequired: false,
  };
  const overview = () => ({
    domain: "aspen.example.com",
    keyFingerprint: "SHA256:this-deployment",
    keyCreatedAt: minutesAgo(9000),
    users: { emigration: gates.usersEmigration, immigration: gates.usersImmigration },
    bots: { emigration: gates.botsEmigration, immigration: gates.botsImmigration },
    usersSharedList: gates.usersSharedList,
    botsSharedList: gates.botsSharedList,
    usersImmigrationInviteRequired: gates.usersImmigrationInviteRequired,
    botsImmigrationInviteRequired: gates.botsImmigrationInviteRequired,
    listsInForce: gates.usersEmigration === "allowList" ? ["usersEmigrationAllow"] : [],
    document: null,
    protocol: { version: 1, minimum: 1, capabilities: [] },
    software: { name: "aspen", version: "0.1.0" },
  });
  return {
    overview,
    update: (change: Record<string, string | boolean>) => {
      Object.assign(gates, change);
      return overview();
    },
    list: (url: URL) => {
      const name = (url.searchParams.get("filter[name]") ?? "").toLowerCase();
      return deployments.filter((d) => d.domain.includes(name));
    },
    add: (body: { domain: string; note?: string }) => {
      const domain = body.domain.trim().toLowerCase();
      if (find(domain) !== undefined) {
        return reply(
          {
            code: "conflict",
            title: "Conflict",
            status: 409,
            detail: "That deployment is already in the directory.",
          },
          409,
        );
      }
      const added = known(domain, {
        note: body.note ?? null,
        publicKey: null,
        publicKeyFingerprint: null,
        firstContactAt: null,
        lastContactAt: null,
      });
      deployments.push(added);
      return reply(added, 201);
    },
    contact: (domain: string) => {
      const deployment = find(domain);
      if (deployment === undefined) {
        return reply({ code: "notFound", title: "Not found", status: 404 }, 404);
      }
      const now = new Date().toISOString();
      const first = deployment.publicKey === null;
      if (first) {
        deployment.publicKey = "Zmlyc3Q";
        deployment.publicKeyFingerprint = "SHA256:first-key";
        deployment.firstContactAt = now;
      }
      deployment.lastContactAt = now;
      const outcome =
        deployment.offeredKey !== null ? "keyChanged" : first ? "pinned" : "confirmed";
      return { outcome, deployment };
    },
    accept: (domain: string, publicKey: string) => {
      const deployment = find(domain);
      if (deployment?.offeredKey !== publicKey) {
        return reply({ code: "conflict", title: "Conflict", status: 409 }, 409);
      }
      deployment.publicKey = deployment.offeredKey;
      deployment.publicKeyFingerprint = deployment.offeredKeyFingerprint;
      deployment.offeredKey = null;
      deployment.offeredKeyFingerprint = null;
      deployment.offeredKeyAt = null;
      return deployment;
    },
    setListed: (domain: string, list: string, on: boolean) => {
      const deployment = find(domain);
      if (deployment === undefined) {
        return reply({ code: "notFound", title: "Not found", status: 404 }, 404);
      }
      const had = deployment.lists.includes(list);
      deployment.lists = on
        ? [...new Set([...deployment.lists, list])]
        : deployment.lists.filter((l) => l !== list);
      deployment.admission = admission(deployment.lists);
      return on ? reply(deployment, had ? 200 : 201) : reply(null, 204);
    },
    forget: (domain: string) => {
      const at = deployments.findIndex((d) => d.domain === domain);
      if (at >= 0) {
        deployments.splice(at, 1);
      }
      return reply(null, 204);
    },
  };
}
