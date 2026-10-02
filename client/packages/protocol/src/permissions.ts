/**
 * Who may do what in a community, resolved the way the server resolves it
 * (`server/src/app/permissions.rs`). Both are tested against `spec/permission_vectors.json`, so
 * they cannot drift apart.
 *
 * A member's permissions across a community are the union of their roles', everyone's role
 * included. In a channel, its category's overrides apply and then the channel's own; within each
 * layer the everyone role's override applies first, then the member's other roles' together,
 * denials before allowances. Overrides touch only channel permissions. The owner holds every
 * permission, whatever the overrides say.
 *
 * The client decides with this only what to offer: the server checks every request itself.
 */

import type { CategoryOverride, ChannelOverride, Permission, Role } from "./generated/events";

export type { Permission };

/** Permissions that hold across a community, in the order the server lists them. */
export const COMMUNITY_PERMISSIONS: readonly Permission[] = [
  "manageCommunity",
  "manageChannels",
  "manageCategories",
  "createInvites",
  "manageInvites",
  "manageRoles",
  "assignRoles",
  "removeMembers",
  "manageMessages",
  "pinMessages",
  "manageCalls",
  "addBots",
  "manageCustomEmoji",
];

/** Permissions a channel or category override may allow or deny. */
export const CHANNEL_PERMISSIONS: readonly Permission[] = [
  "viewChannel",
  "sendMessages",
  "attachFiles",
  "addReactions",
  "startThreads",
  "sendInThreads",
  "createPolls",
  "joinVoice",
  "speak",
  "shareScreen",
  "mentionMembers",
  "mentionRoles",
  "mentionEveryone",
  "transferFiles",
  "useCamera",
];

export const ALL_PERMISSIONS: readonly Permission[] = [
  ...COMMUNITY_PERMISSIONS,
  ...CHANNEL_PERMISSIONS,
];

const CHANNEL_SET: ReadonlySet<Permission> = new Set(CHANNEL_PERMISSIONS);

/** The rank of a community's owner, above every role's position. */
export const OWNER_RANK = 2147483647;
/** A deployment moderator's rank in every community: above every role, below the owner. */
export const MODERATOR_RANK = OWNER_RANK - 1;

/**
 * What Moderate any community gives in every community, as the server's `MODERATION`: seeing
 * everything, and taking things away (messages, attachments, reactions, write-ins, members).
 */
export const MODERATION: readonly Permission[] = ["viewChannel", "manageMessages", "removeMembers"];

/** A set of permissions. */
export type PermissionSet = ReadonlySet<Permission>;

/** Tagging roles and everyone at once, which a new community leaves to its moderators. */
const BROAD_TAGS: ReadonlySet<Permission> = new Set(["mentionRoles", "mentionEveryone"]);

/** The templates a new community's roles start from, as the server writes them. */
export const TEMPLATES = {
  member: [...CHANNEL_PERMISSIONS.filter((p) => !BROAD_TAGS.has(p)), "createInvites"],
  moderator: [
    ...CHANNEL_PERMISSIONS,
    "createInvites",
    "manageInvites",
    "removeMembers",
    "manageMessages",
    "pinMessages",
    "manageCalls",
    "addBots",
    "manageCustomEmoji",
  ],
  admin: ALL_PERMISSIONS,
} as const satisfies Record<string, readonly Permission[]>;

/** An override as the resolver reads it: a role, and what it allows and denies. */
export type OverrideGrant = Pick<ChannelOverride | CategoryOverride, "role" | "allow" | "deny">;

/** What one member, or a deployment moderator, may do across one community. */
export class CommunityPermissions {
  readonly owner: boolean;
  /** Whether they hold Moderate any community, which reaches every community. */
  readonly moderator: boolean;
  /** Every role they hold, everyone's included. */
  readonly roles: readonly Role[];
  readonly held: PermissionSet;
  /** What their roles alone allow. */
  readonly memberHeld: PermissionSet;

  constructor(owner: boolean, roles: readonly Role[], moderator = false) {
    this.owner = owner;
    this.moderator = moderator;
    this.roles = roles;
    this.memberHeld = new Set(owner ? ALL_PERMISSIONS : roles.flatMap((r) => r.permissions));
    this.held = moderator ? new Set([...this.memberHeld, ...MODERATION]) : this.memberHeld;
  }

  /** Whether doing what `permission` allows would be moderation, allowed by nothing else. */
  moderating(permission: Permission): boolean {
    return this.moderator && !this.memberHeld.has(permission);
  }

  has(permission: Permission): boolean {
    return this.held.has(permission);
  }

  /** Their highest role's position, `MODERATOR_RANK` for a moderator, `OWNER_RANK` for the owner. */
  get rank(): number {
    return this.moderator && !this.owner ? MODERATOR_RANK : this.roleRank;
  }

  /** Their rank from the community alone, which is what others act on them by. */
  get roleRank(): number {
    return this.owner ? OWNER_RANK : Math.max(0, ...this.roles.map((r) => r.position));
  }

  /** Whether they may act on a role or member ranked at `position`. */
  outranks(position: number): boolean {
    return position < this.rank;
  }

  /** Their permissions in a channel, after its category's overrides and then its own. */
  inChannel(
    categoryOverrides: readonly OverrideGrant[],
    channelOverrides: readonly OverrideGrant[],
  ): PermissionSet {
    if (this.owner) {
      return this.held;
    }
    const everyone = this.roles.find((r) => r.everyone)?.id;
    const mine = new Set(this.roles.map((r) => r.id));
    const permissions = new Set(this.held);
    for (const layer of [categoryOverrides, channelOverrides]) {
      const base = layer.find((o) => o.role === everyone);
      if (base !== undefined) {
        applyOverride(permissions, base.allow, base.deny);
      }
      const others = layer.filter((o) => o.role !== everyone && mine.has(o.role));
      applyOverride(
        permissions,
        others.flatMap((o) => o.allow),
        others.flatMap((o) => o.deny),
      );
    }
    // No override hides a channel from a moderator.
    if (this.moderator) {
      permissions.add("viewChannel");
    }
    return permissions;
  }
}

/** Denies, then allows, the channel permissions named, in place. */
function applyOverride(
  permissions: Set<Permission>,
  allow: readonly Permission[],
  deny: readonly Permission[],
): void {
  for (const p of deny) {
    if (CHANNEL_SET.has(p)) {
      permissions.delete(p);
    }
  }
  for (const p of allow) {
    if (CHANNEL_SET.has(p)) {
      permissions.add(p);
    }
  }
}

/**
 * What a member may do across a community, from the community's roles, the ones they hold
 * besides everyone's, and whether they own it.
 */
export function resolveCommunity(
  roles: readonly Role[],
  holds: readonly string[] | null,
  owner: boolean,
  moderator = false,
): CommunityPermissions {
  // `null` holds says they are not a member, so not even everyone's role is theirs.
  const held = holds === null ? null : new Set(holds);
  return new CommunityPermissions(
    owner,
    held === null ? [] : roles.filter((r) => r.everyone || held.has(r.id)),
    moderator,
  );
}

/** Every channel permission, which each recipient of a DM holds, and pinning too. */
export const DM_PERMISSIONS: PermissionSet = new Set<Permission>([
  ...CHANNEL_PERMISSIONS,
  "pinMessages",
]);

/** Why a member may or may not do something in a channel, as `explain` finds it. */
export type AccessReason =
  | { readonly kind: "owner" }
  | { readonly kind: "moderator" }
  | { readonly kind: "roles"; readonly roles: readonly string[] }
  | { readonly kind: "none" }
  | {
      readonly kind: "override";
      readonly layer: "category" | "channel";
      readonly role: string;
      readonly allowed: boolean;
    };

export interface AccessDecision {
  readonly allowed: boolean;
  readonly reason: AccessReason;
}

/**
 * The same decision `inChannel` makes for one permission, with what made it: the owner, the
 * roles that grant it, or the last override that allowed or denied it.
 */
export function explain(
  access: CommunityPermissions,
  permission: Permission,
  categoryOverrides: readonly OverrideGrant[],
  channelOverrides: readonly OverrideGrant[],
): AccessDecision {
  if (access.owner) {
    return { allowed: true, reason: { kind: "owner" } };
  }
  const granting = access.roles.filter((r) => r.permissions.includes(permission)).map((r) => r.id);
  let decision: AccessDecision =
    granting.length > 0
      ? { allowed: true, reason: { kind: "roles", roles: granting } }
      : { allowed: false, reason: { kind: "none" } };
  const moderated = (d: AccessDecision): AccessDecision =>
    !d.allowed && access.moderator && MODERATION.includes(permission)
      ? { allowed: true, reason: { kind: "moderator" } }
      : d;
  if (!CHANNEL_SET.has(permission)) {
    return moderated(decision);
  }
  const everyone = access.roles.find((r) => r.everyone)?.id;
  const mine = new Set(access.roles.map((r) => r.id));
  const layers = [
    ["category", categoryOverrides],
    ["channel", channelOverrides],
  ] as const;
  for (const [layer, overrides] of layers) {
    const decide = (o: OverrideGrant, allowed: boolean): AccessDecision => ({
      allowed,
      reason: { kind: "override", layer, role: o.role, allowed },
    });
    const base = overrides.find((o) => o.role === everyone);
    if (base?.deny.includes(permission) === true) {
      decision = decide(base, false);
    }
    if (base?.allow.includes(permission) === true) {
      decision = decide(base, true);
    }
    const others = overrides.filter((o) => o.role !== everyone && mine.has(o.role));
    const denying = others.find((o) => o.deny.includes(permission));
    if (denying !== undefined) {
      decision = decide(denying, false);
    }
    const allowing = others.find((o) => o.allow.includes(permission));
    if (allowing !== undefined) {
      decision = decide(allowing, true);
    }
  }
  return moderated(decision);
}
