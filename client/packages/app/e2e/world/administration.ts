import { federationWorld } from "./federation";
import { bob, community, deploymentAdministrator, me, minutesAgo } from "./fixtures";
import { reply } from "./reply";

/** A registration invite of the world's, as the admin API lists it. */
interface WorldInvite {
  code: string;
  createdBy: string | null;
  createdAt: string;
  expiresAt: string | null;
  maxUses: number;
  uses: number;
  revokedAt: string | null;
  note: string | null;
  usable: boolean;
}

/** The invite already made from the terminal, used once of its two uses. */
export const standingInvite = "Terminal01";

/**
 * The Administration Dashboard's side of the world: the caller administers it; its invites
 * change as a spec makes and revokes them, one set per page.
 */
export function administration() {
  const invites: WorldInvite[] = [
    {
      code: standingInvite,
      createdBy: null,
      createdAt: minutesAgo(600),
      expiresAt: null,
      maxUses: 2,
      uses: 1,
      revokedAt: null,
      note: "for the family",
      usable: true,
    },
  ];
  const directory = [
    {
      id: bob,
      name: "bob",
      displayName: "Bob With A Rather Long Display Name",
      icon: null,
      createdAt: minutesAgo(900),
      roles: [] as string[],
      registeredWith: standingInvite,
    },
    {
      id: me,
      name: "kate",
      displayName: "Kate",
      icon: null,
      createdAt: minutesAgo(1200),
      roles: [deploymentAdministrator],
      registeredWith: null,
    },
  ];
  // Twenty more people who joined a day apart, so the list runs to more than one page.
  for (let i = 1; i <= 20; i++) {
    directory.push({
      id: `0190f0a0-0000-7000-8000-0000000001${String(i).padStart(2, "0")}`,
      name: `member${String(i).padStart(2, "0")}`,
      displayName: `Member ${String(i).padStart(2, "0")}`,
      icon: null,
      createdAt: minutesAgo(1440 * i + 1500),
      roles: [] as string[],
      registeredWith: standingInvite,
    });
  }
  const communities = [
    { id: community, name: "Family", icon: null, members: 2, createdAt: minutesAgo(3000) },
    {
      id: "0190f0a0-0000-7000-8000-000000000020",
      name: "Book club",
      icon: null,
      members: 9,
      createdAt: minutesAgo(9000),
    },
  ];
  const named = (url: URL) => (url.searchParams.get("filter[name]") ?? "").toLowerCase();
  /** A page of `rows` as the server gives it: sorted by `sort`, from `offset`, `limit` long. */
  function page<T extends Record<string, unknown>>(
    rows: T[],
    url: URL,
    keyOf: (row: T, field: string) => string | number,
  ): T[] {
    const sort = url.searchParams.get("sort") ?? "-createdAt";
    const field = sort.replace(/^-/, "");
    const sign = sort.startsWith("-") ? -1 : 1;
    const sorted = [...rows].sort((a, b) => {
      const x = keyOf(a, field);
      const y = keyOf(b, field);
      return (x < y ? -1 : x > y ? 1 : 0) * sign;
    });
    const offset = Number(url.searchParams.get("offset") ?? "0");
    const limit = Number(url.searchParams.get("limit") ?? "15");
    return sorted.slice(offset, offset + limit);
  }
  // How the deployment presents itself, as `GET /deployment` answers and its `PATCH` changes.
  const profile: { displayName: string | null; icon: null } = {
    displayName: "Family Server",
    icon: null,
  };
  // The deployment's policies, as `GET /admin/settings` answers and its `PATCH` changes.
  const settings: Record<string, number | boolean> = {
    registrationInviteRequired: true,
    requireTwoFactor: false,
    botsEnabled: true,
    botsMaxPerUser: 25,
    everyoneMentionLimit: 200,
    customEmojiLimit: 1000,
    evidenceRetentionDays: 365,
    fileTransfers: true,
  };
  return {
    profile: () => profile,
    settings: () => settings,
    updateSettings: (change: Record<string, number | boolean>) => Object.assign(settings, change),
    updateProfile: (change: { displayName?: string | null }) => {
      if (change.displayName !== undefined) {
        profile.displayName = change.displayName;
      }
      return profile;
    },
    overview: () => ({
      users: 22,
      newUsersThisWeek: 2,
      communities: 2,
      registrationInviteRequired: true,
    }),
    users: (url: URL) =>
      page(
        directory.filter((u) =>
          [u.name, u.displayName].some((n) => n.toLowerCase().includes(named(url))),
        ),
        url,
        (u, field) => (field === "name" ? u.displayName.toLowerCase() : u.createdAt),
      ),
    communities: (url: URL) =>
      page(
        communities.filter((c) => c.name.toLowerCase().includes(named(url))),
        url,
        (c, field) =>
          field === "name" ? c.name.toLowerCase() : field === "members" ? c.members : c.createdAt,
      ),
    growth: (url: URL) => {
      const monthly = url.searchParams.get("range") === "fiveYears";
      const steps = monthly ? 61 : 92;
      return {
        unit: monthly ? "month" : "day",
        points: Array.from({ length: steps }, (_, i) => {
          const back = steps - 1 - i;
          const at = new Date(Date.now() - back * (monthly ? 30 : 1) * 86_400_000);
          return {
            at: at.toISOString(),
            users: Math.round((monthly ? 4 : 380) + i * (monthly ? 6 : 0.4)),
            communities: Math.round((monthly ? 1 : 30) + i * (monthly ? 0.5 : 0.08)),
          };
        }),
      };
    },
    invites: () => invites,
    federation: federationWorld(),
    create: (body: { maxUses?: number; note?: string }) => {
      const invite: WorldInvite = {
        code: `Made${String(invites.length).padStart(4, "0")}`,
        createdBy: me,
        createdAt: new Date().toISOString(),
        expiresAt: null,
        maxUses: body.maxUses ?? 1,
        uses: 0,
        revokedAt: null,
        note: body.note ?? null,
        usable: true,
      };
      invites.unshift(invite);
      return reply(invite, 201);
    },
    revoke: (code: string) => {
      const invite = invites.find((i) => i.code === code);
      if (invite !== undefined) {
        invite.revokedAt = new Date().toISOString();
        invite.usable = false;
      }
      return reply(null, 204);
    },
    fleet: () => ({
      apiServers: [
        {
          instance: "a",
          host: "api-1",
          version: "0.1.0",
          startedAt: minutesAgo(60 * 26),
          reportedAt: minutesAgo(0),
          eventStreams: 42,
          requestsPerMinute: 318.5,
          serverErrorsPerMinute: 0,
          residentBytes: 214 * 1024 * 1024,
          dbConnections: 8,
          dbConnectionsIdle: 6,
        },
        {
          instance: "b",
          host: "api-2",
          version: "0.1.0",
          startedAt: minutesAgo(90),
          reportedAt: minutesAgo(0),
          eventStreams: 17,
          requestsPerMinute: 120,
          serverErrorsPerMinute: 2.5,
          residentBytes: 180 * 1024 * 1024,
          dbConnections: 4,
          dbConnectionsIdle: 4,
        },
      ],
      voiceServers: [
        {
          id: "0190f0a0-0000-7000-8000-000000000031",
          name: "voice-east",
          url: "https://voice-east.example",
          enabled: true,
          capacity: 500,
          participants: 12,
          lastReportAt: minutesAgo(0),
          reporting: true,
        },
        {
          id: "0190f0a0-0000-7000-8000-000000000032",
          name: "voice-west",
          url: "https://voice-west.example",
          enabled: true,
          capacity: 500,
          participants: 0,
          lastReportAt: minutesAgo(45),
          reporting: false,
        },
        {
          id: "0190f0a0-0000-7000-8000-000000000033",
          name: "voice-spare",
          url: "https://voice-spare.example",
          enabled: false,
          capacity: 100,
          participants: 0,
          lastReportAt: null,
          reporting: false,
        },
      ],
      unregisteredVoiceServers: [
        { id: "0190f0a0-0000-7000-8000-000000000099", reportedAt: minutesAgo(0) },
      ],
    }),
  };
}
