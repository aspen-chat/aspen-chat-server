import {
  Navigate,
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
import { Home } from "@/features/home/Home";
import { InviteScreen } from "@/features/invites/InviteScreen";
import { RootLayout } from "@/features/layout/RootLayout";
import { ChannelScreen } from "@/features/messages/ChannelScreen";
import { NotFound } from "@/features/layout/NotFound";

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
 *   /invite/{code}                                      what an invite link opens: join or open
 *   /register?invite={code}                             create an account with a registration
 *                                                       invite; signed in, the home screen
 *   /admin                                              the Administration Dashboard
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

export const inviteRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/invite/$code",
  component: InviteScreen,
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

export const communityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/communities/$communityId",
  component: ChannelSidebarLayout,
});

export const communityIndexRoute = createRoute({
  getParentRoute: () => communityRoute,
  path: "/",
  component: CommunityIndex,
});

export const channelRoute = createRoute({
  getParentRoute: () => communityRoute,
  path: "/channels/$channelId",
  component: ChannelScreen,
});

export const messageRoute = createRoute({
  getParentRoute: () => communityRoute,
  path: "/channels/$channelId/messages/$messageId",
  component: ChannelScreen,
});

export const threadRoute = createRoute({
  getParentRoute: () => communityRoute,
  path: "/channels/$channelId/threads/$threadId",
  component: ChannelScreen,
});

export const dmsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dms",
  component: DmLayout,
});

export const dmsIndexRoute = createRoute({
  getParentRoute: () => dmsRoute,
  path: "/",
  component: DmIndex,
});

export const dmRoute = createRoute({
  getParentRoute: () => dmsRoute,
  path: "/$channelId",
  component: ChannelScreen,
});

export const dmMessageRoute = createRoute({
  getParentRoute: () => dmsRoute,
  path: "/$channelId/messages/$messageId",
  component: ChannelScreen,
});

export const dmThreadRoute = createRoute({
  getParentRoute: () => dmsRoute,
  path: "/$channelId/threads/$threadId",
  component: ChannelScreen,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  inviteRoute,
  registerRoute,
  adminRoute,
  communityRoute.addChildren([communityIndexRoute, channelRoute, messageRoute, threadRoute]),
  dmsRoute.addChildren([dmsIndexRoute, dmRoute, dmMessageRoute, dmThreadRoute]),
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
