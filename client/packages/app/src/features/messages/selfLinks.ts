import { linkOptions } from "@tanstack/react-router";
import { deviceLinkOf } from "@/features/qr/aspenLinks";
import {
  channelLink,
  communityLink,
  deploymentLink,
  dmsLink,
  inviteLink,
  messageLink,
  threadLink,
  type ChannelHome,
  type Domain,
} from "@/features/messages/links";

/** A route of the app, as a link to it names it. */
export type SelfRoute =
  | { readonly kind: "deployment" }
  | { readonly kind: "community"; readonly community: string }
  | ({ readonly kind: "channel" } & Place)
  | ({ readonly kind: "message"; readonly message: string } & Place)
  | ({ readonly kind: "thread"; readonly thread: string } & Place)
  | { readonly kind: "dms" }
  | { readonly kind: "invite"; readonly code: string }
  | { readonly kind: "registration"; readonly code: string }
  | { readonly kind: "deviceLink"; readonly server: string; readonly id: string }
  | { readonly kind: "admin"; readonly tab: string | null }
  | { readonly kind: "botAdd"; readonly bot: string; readonly permissions: string | null }
  | { readonly kind: "attributions" };

/** A channel or DM, as its routes name it: its community, or `null` for a DM. */
interface Place {
  readonly community: string | null;
  readonly channel: string;
}

/** A link to a deployment the user uses: which one (`null` for the home), and where in it. */
export interface SelfLinkTarget {
  readonly domain: Domain;
  readonly route: SelfRoute;
}

/** The addresses a link may name the deployments the user uses by. */
export interface KnownHosts {
  /** The home's: the address the app reaches it at, and the page's own. */
  readonly home: readonly string[];
  /** The other deployments the user signs in to from home, by domain. */
  readonly foreign: readonly string[];
}

const UUID = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
const CODE = "[A-Za-z0-9]{1,16}";
const COMMUNITY = new RegExp(`^/communities/(${UUID})$`);
const CHANNEL = new RegExp(
  `^/communities/(${UUID})/channels/(${UUID})(?:/(messages|threads)/(${UUID}))?$`,
);
const DM = new RegExp(`^/dms/(${UUID})(?:/(messages|threads)/(${UUID}))?$`);
const INVITE = new RegExp(`^/invite/(${CODE})$`);
const ADMIN = /^\/admin(?:\/([A-Za-z]+))?$/;
const BOT_ADD = new RegExp(`^/bots/(${UUID})/add$`);
const ABROAD = /^\/at\/([^/]+)(\/.*)?$/;

/**
 * Where a link leads when it is to a deployment the user uses (`hosts`), by the routes of the
 * app (`router.tsx`): a deployment's own routes on its address, or on the home's under
 * `/at/{domain}`, and the home's other pages (the dashboard, adding a bot, and the rest) on
 * the home's address. `null` for any other link, the deployment's API and files included, and
 * for an invite whose `?at=` names a deployment the user does not use.
 */
export function parseSelfLink(href: string, hosts: KnownHosts): SelfLinkTarget | null {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    return null;
  }
  const host = url.host.toLowerCase();
  const path = url.pathname.length > 1 ? url.pathname.replace(/\/$/, "") : "/";
  const domainOf = (name: string): Domain | undefined =>
    hosts.home.includes(name) ? null : hosts.foreign.includes(name) ? name : undefined;
  const domain = domainOf(host);
  if (domain === undefined) {
    return null;
  }
  const abroad = domain === null ? ABROAD.exec(path) : null;
  if (abroad !== null) {
    const named = decodedDomain(abroad[1] ?? "");
    const there = named === null ? undefined : domainOf(named);
    if (there === undefined) {
      return null;
    }
    const route = deploymentRoute(abroad[2] ?? "/");
    return route === null ? null : withInviteDomain({ domain: there, route }, url, domainOf);
  }
  const route =
    deploymentRoute(path) ?? (domain === null ? homeRoute(path, url.search, url.hash) : null);
  return route === null ? null : withInviteDomain({ domain, route }, url, domainOf);
}

/** A route every deployment has, at the home's root or under `/at/{domain}`. */
function deploymentRoute(path: string): SelfRoute | null {
  if (path === "/") {
    return { kind: "deployment" };
  }
  if (path === "/dms") {
    return { kind: "dms" };
  }
  const community = COMMUNITY.exec(path);
  if (community?.[1] !== undefined) {
    return { kind: "community", community: community[1] };
  }
  const channel = CHANNEL.exec(path);
  if (channel?.[1] !== undefined && channel[2] !== undefined) {
    return placeRoute({ community: channel[1], channel: channel[2] }, channel[3], channel[4]);
  }
  const dm = DM.exec(path);
  if (dm?.[1] !== undefined) {
    return placeRoute({ community: null, channel: dm[1] }, dm[2], dm[3]);
  }
  const invite = INVITE.exec(path);
  if (invite?.[1] !== undefined) {
    return { kind: "invite", code: invite[1] };
  }
  return null;
}

function placeRoute(place: Place, sub: string | undefined, id: string | undefined): SelfRoute {
  if (sub === "messages" && id !== undefined) {
    return { kind: "message", ...place, message: id };
  }
  if (sub === "threads" && id !== undefined) {
    return { kind: "thread", ...place, thread: id };
  }
  return { kind: "channel", ...place };
}

/** A route only the home has, at its root. */
function homeRoute(path: string, query: string, fragment: string): SelfRoute | null {
  const search = new URLSearchParams(query);
  if (path === "/register") {
    const code = search.get("invite") ?? "";
    return new RegExp(`^${CODE}$`).test(code) ? { kind: "registration", code } : null;
  }
  if (path === "/device-link") {
    const link = deviceLinkOf(query.replace(/^\?/, ""), fragment.replace(/^#/, ""));
    return link === null ? null : { kind: "deviceLink", ...link };
  }
  if (path === "/attributions") {
    return { kind: "attributions" };
  }
  const admin = ADMIN.exec(path);
  if (admin !== null) {
    return { kind: "admin", tab: admin[1] ?? null };
  }
  const bot = BOT_ADD.exec(path);
  if (bot?.[1] !== undefined) {
    return { kind: "botAdd", bot: bot[1], permissions: search.get("permissions") };
  }
  return null;
}

/**
 * An invite's `?at=`, which names the deployment it belongs to whatever address the link is
 * on, decides where it leads; one the user does not use leaves the link to the invite screen.
 */
function withInviteDomain(
  target: SelfLinkTarget,
  url: URL,
  domainOf: (name: string) => Domain | undefined,
): SelfLinkTarget | null {
  const at = url.searchParams.get("at");
  if (target.route.kind !== "invite" || at === null) {
    return target;
  }
  const named = decodedDomain(at);
  const domain = named === null ? undefined : domainOf(named);
  return domain === undefined ? null : { ...target, domain };
}

function decodedDomain(encoded: string): string | null {
  try {
    const domain = decodeURIComponent(encoded).trim().toLowerCase();
    return domain === "" ? null : domain;
  } catch {
    return null;
  }
}

/** The route a link of the deployment leads to, opened in place. */
export function selfLinkRoute({ domain, route }: SelfLinkTarget) {
  const home = (community: string | null): ChannelHome => ({ domain, community });
  switch (route.kind) {
    case "deployment":
      return deploymentLink(domain);
    case "community":
      return communityLink(domain, route.community);
    case "channel":
      return channelLink(home(route.community), route.channel);
    case "message":
      return messageLink(home(route.community), route.channel, route.message);
    case "thread":
      return threadLink(home(route.community), route.channel, route.thread);
    case "dms":
      return dmsLink(domain);
    case "invite":
      return inviteLink(domain, route.code);
    case "registration":
      return linkOptions({ to: "/register", search: { invite: route.code } });
    case "deviceLink":
      return linkOptions({ to: "/device-link", search: { server: route.server, link: route.id } });
    case "admin":
      return route.tab === null
        ? linkOptions({ to: "/admin" })
        : linkOptions({ to: "/admin/$tab", params: { tab: route.tab } });
    case "botAdd":
      return linkOptions({
        to: "/bots/$botId/add",
        params: { botId: route.bot },
        search: route.permissions === null ? {} : { permissions: route.permissions },
      });
    case "attributions":
      return linkOptions({ to: "/attributions" });
  }
}
