import {
  createBrowserHistory,
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { detectShell } from "@/config";
import { ChannelSidebarLayout, CommunityIndex } from "@/features/channels/CommunityScreen";
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
 *   /invite/{code}                                      what an invite link opens: join or open
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

const routeTree = rootRoute.addChildren([
  indexRoute,
  inviteRoute,
  communityRoute.addChildren([communityIndexRoute, channelRoute, messageRoute]),
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
