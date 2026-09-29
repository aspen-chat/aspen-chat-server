import {
  Navigate,
  Outlet,
  createBrowserHistory,
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { detectShell } from "@/config";
import { AdminDashboard } from "@/features/admin/AdminDashboard";
import { ChannelSidebarLayout, CommunityIndex } from "@/features/channels/CommunityScreen";
import { DmIndex, DmLayout } from "@/features/dms/DmLayout";
import { ForeignIndex, Home } from "@/features/home/Home";
import { InviteScreen } from "@/features/invites/InviteScreen";
import { RootLayout } from "@/features/layout/RootLayout";
import { ChannelScreen } from "@/features/messages/ChannelScreen";
import { NotFound } from "@/features/layout/NotFound";
import { ForeignScope } from "@/api/deployments";
import { BotAddScreen } from "@/features/bots/BotAddScreen";

/**
 * URL scheme, shared by every shell so a link copied from one opens in another:
 *
 *   /                                                   the first community, or an empty state
 *   /communities/{community}                            the community's channel list
 *   /communities/{community}/channels/{channel}         a channel's history
 *   /communities/{community}/channels/{channel}/messages/{message}
 *                                                       the same, opened around one message
 *   /communities/{community}/channels/{channel}/threads/{thread}
 *                                                       the same, with a thread open beside it
 *   /dms                                                the caller's DMs and group DMs
 *   /dms/{channel}, /dms/{channel}/messages/{message}, /dms/{channel}/threads/{thread}
 *                                                       a DM, as a channel is above
 *   /invite/{code}?at={domain}                          what an invite link opens: join or open;
 *                                                       `at` names the invite's deployment, and
 *                                                       one other than the home redirects to
 *                                                       `/at/{domain}/invite/{code}`
 *   /register?invite={code}                             create an account with a registration
 *                                                       invite; signed in, the home screen
 *   /admin                                              the Administration Dashboard
 *   /bots/{bot}/add?permissions={names}                 what a bot's link opens: add it to a
 *                                                       community, with the permissions named
 *   /at/{domain}/communities/…, /at/{domain}/dms/…, /at/{domain}/invite/{code}
 *                                                       the same on another deployment the user
 *                                                       signs in to from home, named by its
 *                                                       domain (with `:port` when not 443)
 *
 * The web build uses real paths. Electron loads the bundle from `file://` and Capacitor from
 * an app-local origin, where the server cannot rewrite deep links to `index.html`, so those
 * shells keep the same routes after a `#`.
 */
export const rootRoute = createRootRoute({
  component: RootLayout,
  notFoundComponent: NotFound,
});

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: Home,
});

export const registerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/register",
  validateSearch: (search: Record<string, unknown>): { invite?: string } =>
    typeof search.invite === "string" ? { invite: search.invite } : {},
  // Signed out, the root layout shows the create-account screen; signed in, there is nothing
  // to register.
  component: () => <Navigate to="/" replace />,
});

export const adminRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/admin",
  component: AdminDashboard,
});

export const botAddRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/bots/$botId/add",
  validateSearch: (search: Record<string, unknown>): { permissions?: string } =>
    typeof search.permissions === "string" ? { permissions: search.permissions } : {},
  component: function BotAdd() {
    const { botId } = botAddRoute.useParams();
    const { permissions } = botAddRoute.useSearch();
    return <BotAddScreen botId={botId} permissions={permissions} />;
  },
});

// A deployment's routes: its invites, communities, and DMs. The user's home has them at the
// root and every other deployment under `/at/$domain`; the components read their parameters
// without naming a route, so the same ones serve both. The two sets are written out rather
// than made by one function over the parent route, because the router types each path from its
// parent's literal type, and a parent passed as a type parameter leaves every path untyped.

export const inviteRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/invite/$code",
  validateSearch: (search: Record<string, unknown>): { at?: string } =>
    typeof search.at === "string" ? { at: search.at } : {},
  component: InviteScreen,
});

export const communityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/communities/$communityId",
  component: ChannelSidebarLayout,
});

const communityIndexRoute = createRoute({
  getParentRoute: () => communityRoute,
  path: "/",
  component: CommunityIndex,
});

const channelRoute = createRoute({
  getParentRoute: () => communityRoute,
  path: "/channels/$channelId",
  component: ChannelScreen,
});

const messageRoute = createRoute({
  getParentRoute: () => communityRoute,
  path: "/channels/$channelId/messages/$messageId",
  component: ChannelScreen,
});

const threadRoute = createRoute({
  getParentRoute: () => communityRoute,
  path: "/channels/$channelId/threads/$threadId",
  component: ChannelScreen,
});

export const dmsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dms",
  component: DmLayout,
});

const dmsIndexRoute = createRoute({
  getParentRoute: () => dmsRoute,
  path: "/",
  component: DmIndex,
});

const dmRoute = createRoute({
  getParentRoute: () => dmsRoute,
  path: "/$channelId",
  component: ChannelScreen,
});

const dmMessageRoute = createRoute({
  getParentRoute: () => dmsRoute,
  path: "/$channelId/messages/$messageId",
  component: ChannelScreen,
});

const dmThreadRoute = createRoute({
  getParentRoute: () => dmsRoute,
  path: "/$channelId/threads/$threadId",
  component: ChannelScreen,
});

/** Another deployment the user signs in to from home: its routes run with its client and sync. */
export const foreignRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/at/$domain",
  component: function Foreign() {
    const { domain } = foreignRoute.useParams();
    return (
      <ForeignScope domain={domain}>
        <Outlet />
      </ForeignScope>
    );
  },
});

const foreignIndexRoute = createRoute({
  getParentRoute: () => foreignRoute,
  path: "/",
  component: ForeignIndex,
});

const foreignInviteRoute = createRoute({
  getParentRoute: () => foreignRoute,
  path: "/invite/$code",
  component: InviteScreen,
});

const foreignCommunityRoute = createRoute({
  getParentRoute: () => foreignRoute,
  path: "/communities/$communityId",
  component: ChannelSidebarLayout,
});

const foreignCommunityIndexRoute = createRoute({
  getParentRoute: () => foreignCommunityRoute,
  path: "/",
  component: CommunityIndex,
});

const foreignChannelRoute = createRoute({
  getParentRoute: () => foreignCommunityRoute,
  path: "/channels/$channelId",
  component: ChannelScreen,
});

const foreignMessageRoute = createRoute({
  getParentRoute: () => foreignCommunityRoute,
  path: "/channels/$channelId/messages/$messageId",
  component: ChannelScreen,
});

const foreignThreadRoute = createRoute({
  getParentRoute: () => foreignCommunityRoute,
  path: "/channels/$channelId/threads/$threadId",
  component: ChannelScreen,
});

const foreignDmsRoute = createRoute({
  getParentRoute: () => foreignRoute,
  path: "/dms",
  component: DmLayout,
});

const foreignDmsIndexRoute = createRoute({
  getParentRoute: () => foreignDmsRoute,
  path: "/",
  component: DmIndex,
});

const foreignDmRoute = createRoute({
  getParentRoute: () => foreignDmsRoute,
  path: "/$channelId",
  component: ChannelScreen,
});

const foreignDmMessageRoute = createRoute({
  getParentRoute: () => foreignDmsRoute,
  path: "/$channelId/messages/$messageId",
  component: ChannelScreen,
});

const foreignDmThreadRoute = createRoute({
  getParentRoute: () => foreignDmsRoute,
  path: "/$channelId/threads/$threadId",
  component: ChannelScreen,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  registerRoute,
  adminRoute,
  botAddRoute,
  inviteRoute,
  communityRoute.addChildren([communityIndexRoute, channelRoute, messageRoute, threadRoute]),
  dmsRoute.addChildren([dmsIndexRoute, dmRoute, dmMessageRoute, dmThreadRoute]),
  foreignRoute.addChildren([
    foreignIndexRoute,
    foreignInviteRoute,
    foreignCommunityRoute.addChildren([
      foreignCommunityIndexRoute,
      foreignChannelRoute,
      foreignMessageRoute,
      foreignThreadRoute,
    ]),
    foreignDmsRoute.addChildren([
      foreignDmsIndexRoute,
      foreignDmRoute,
      foreignDmMessageRoute,
      foreignDmThreadRoute,
    ]),
  ]),
]);

export const router = createRouter({
  routeTree,
  history: detectShell() === "web" ? createBrowserHistory() : createHashHistory(),
  defaultPreload: false,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
