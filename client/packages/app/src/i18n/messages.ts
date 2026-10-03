import { access } from "./en/access";
import { addOptions } from "./en/addOptions";
import { admin } from "./en/admin";
import { blocking } from "./en/blocking";
import { bots } from "./en/bots";
import { channelActions } from "./en/channelActions";
import { commands } from "./en/commands";
import { communitySettings } from "./en/communitySettings";
import { crop } from "./en/crop";
import { deploymentPermissionNames } from "./en/deploymentPermissionNames";
import { deployments } from "./en/deployments";
import { dms } from "./en/dms";
import { emoji } from "./en/emoji";
import { emojiPanel } from "./en/emojiPanel";
import { expiry } from "./en/expiry";
import { federation } from "./en/federation";
import { files } from "./en/files";
import { folders } from "./en/folders";
import { gallery } from "./en/gallery";
import { layout } from "./en/layout";
import { members } from "./en/members";
import { mute } from "./en/mute";
import { notifications } from "./en/notifications";
import { palettes } from "./en/palettes";
import { permissionGroups } from "./en/permissionGroups";
import { permissionNames } from "./en/permissionNames";
import { pins } from "./en/pins";
import { poll } from "./en/poll";
import { profile } from "./en/profile";
import { roles } from "./en/roles";
import { search } from "./en/search";
import { security } from "./en/security";
import { settings } from "./en/settings";
import { status } from "./en/status";
import { syncStatus } from "./en/syncStatus";
import { system } from "./en/system";
import { tagging } from "./en/tagging";
import { themeModes } from "./en/themeModes";
import { threads } from "./en/threads";
import { twoFactor } from "./en/twoFactor";
import { voice } from "./en/voice";

/**
 * User-facing strings. Keys are camelCase to match the server's `locales/*.yml` convention.
 * Server-produced Problem `title`/`detail` values are already localized and are shown as-is.
 */
export const en = {
  appName: "Aspen",
  loginHeading: "Sign in",
  welcomeNamed: "Welcome to {name}, an Aspen Chat instance.",
  welcomeUnnamed: "Welcome to our Aspen Chat instance.",
  welcomeToAspenChat: "Welcome to Aspen Chat.",
  deploymentUrlLabel: "Deployment URL",
  backToSignIn: "Back",
  inviteCodeLabel: "Invite code",
  inviteCodeRequiredHint:
    "This server takes new accounts by invite. Ask its administrators for a code.",
  inviteCodeHint: "The code from your invite link.",
  registerHeading: "Create an account",
  noAccountYet: "No account yet?",
  createAccount: "Create one",
  haveAccount: "Already have an account?",
  signInInstead: "Sign in",
  confirmPasswordLabel: "Confirm password",
  passwordHint: "At least 8 characters.",
  passwordsDoNotMatch: "The passwords do not match.",
  register: "Create account",
  registering: "Creating account…",
  serverLabel: "Server",
  serverPlaceholder: "chat.example.org",
  usernameLabel: "Username",
  passwordLabel: "Password",
  signIn: "Sign in",
  signingIn: "Signing in…",
  signOut: "Sign out",
  signInWithPasskey: "Sign in with a passkey",
  orDivider: "or",
  passkeyWaiting: "Waiting for your passkey…",
  passkeyWaitingBrowser: "Continue in your browser…",
  deployments,
  twoFactor,
  security,
  crop,
  changeCommunityIcon: "Change community icon",
  settings,
  files,
  voice,
  profile,
  emojiPanel,
  emoji,
  tagging,
  layout,
  system,
  commands,
  bots,
  blocking,
  threads,
  dms,
  changeServer: "Change",
  continue: "Continue",
  invalidServerUrl: "Enter a deployment URL such as chat.example.org.",
  loading: "Loading…",
  retry: "Retry",
  notFoundHeading: "There is nothing here.",
  backHome: "Back to your communities",
  communitiesLabel: "Communities",
  unreadLabel: "{name}, unread",
  oneMention: "1 mention",
  unknownRole: "unknown role",
  mentions: "{count} mentions",
  withMentions: "{name}, {mentions}",
  mutedLabel: "{name}, muted",
  admin,
  mute,
  channelsLabel: "Channels",
  noCommunitiesHeading: "No communities yet",
  noCommunitiesHint: "Create a community of your own, or join one with an invite link.",
  createCommunity: "Create a community",
  communityNameLabel: "Community name",
  create: "Create",
  creating: "Creating…",
  orJoin: "or join one you were invited to",
  addCommunity: "Create or join a community",
  createCommunityHint: "Start a new place and invite people to it.",
  joinCommunityHint: "Use an invite link or code someone shared with you.",
  back: "Back",
  addNew: "Add new…",
  addOptions,
  newChannel: "New channel",
  addToCategory: "Add a channel to {category}",
  newChannelIn: "New channel in {category}",
  newTextChannelIn: "New text channel in {category}",
  newVoiceChannelIn: "New voice channel in {category}",
  newTextChannel: "New text channel",
  newVoiceChannel: "New voice channel",
  newCategory: "New category",
  categoryNameLabel: "Category name",
  channelNameLabel: "Channel name",
  categoryLabel: "Category",
  noCategory: "No category",
  joinCommunity: "Join a community",
  inviteInputLabel: "Invite link or code",
  inviteInputInvalid: "That does not look like an invite link or code.",
  lookingUpInvite: "Checking the invite…",
  inviteUnusableHeading: "This invite cannot be used",
  inviteNotFound:
    "It doesn't exist here: it may have been mistyped or revoked. Ask whoever shared it for a new one.",
  inviteOnDomain:
    "This invite is to a community on {domain}. Sign in, and it opens next; your account doesn't need to be on {domain}.",
  inviteExpiredHeading: "This invite has expired",
  inviteExpiredHint: "Ask for a new one from someone in the community.",
  invitedTo: "You have been invited to {community}",
  alreadyMember: "You are already a member of {community}",
  join: "Join",
  joining: "Joining…",
  openCommunity: "Open",
  invitePeople: "Invite people",
  inviteDialogHeading: "Invite people to {community}",
  expiryLabel: "Expires",
  expiry,
  createInvite: "Create invite",
  creatingInvite: "Creating…",
  noInvitesYet: "No invites yet. Create one and share the link.",
  neverExpires: "Never expires",
  expiresOn: "Expires {date}",
  expired: "Expired",
  copyLink: "Copy link",
  copied: "Copied",
  copyFailed: "This browser wouldn't copy it. Here it is to copy yourself:",
  revoke: "Revoke",
  close: "Close",
  edited: "(edited)",
  editedAt: "Edited {date}",
  edit: "Edit",
  editMessage: "Edit message",
  delete: "Delete",
  deleteMessage: "Delete message",
  save: "Save",
  cancel: "Cancel",
  editingHint: "Enter to save, Escape to cancel",
  editMessageLabel: "Edit message",
  deleteMessageHeading: "Delete this message?",
  deleteMessageHint: "It will be removed for everyone.",
  deleting: "Deleting…",
  messageActionsLabel: "Message actions",
  /** A finger held on a message, where the actions are offered that way. */
  longPressForActions: "Press and hold for the message's actions",
  copyMessageText: "Copy text",
  copiedMessageText: "Copied text",
  toastsLabel: "Notifications",
  react: "React",
  addReaction: "Add a reaction",
  reactionsLabel: "Reactions",
  emojiSearch: "Search emoji",
  reactedBy: "{names} reacted with {emoji}",
  reactedByMore: "{names} … and {count} more reacted with {emoji}",
  moreReactions: "{count} more reactions",
  viewReactions: "View reactions",
  reactionsHeading: "Reactions",
  noReactions: "No reactions yet.",
  reactionCount: "{emoji}, {count}",
  reactedWithLabel: "Reacted with {emoji}",
  showMore: "Show more",
  youReactedWith: "Remove your {emoji}",
  reactWith: "React with {emoji}",
  membersLabel: "Members",
  categoryChannelsLabel: "{category} channels",
  dragChannel: "Drag {channel}",
  emptyChannelGroup: "No channels yet; drop one here",
  dragCommunity: "Drag {community}",
  showMembers: "Show members",
  hideMembers: "Hide members",
  onlineGroup: "Online — {count}",
  offlineGroup: "Offline — {count}",
  status,
  noChannels: "This community has no text channels yet.",
  communityNotFound: "That community is not one you belong to, or it no longer exists.",
  channelNotFound: "That channel does not exist or was deleted.",
  backToChannels: "Back to channels",
  channelStart: "This is the beginning of the channel.",
  jumpToLatest: "Jump to latest",
  jumpingToLatest: "Loading the latest…",
  newMessages: "New Messages",
  messageLabel: "Message",
  messagePlaceholder: "Message #{channel}",
  send: "Send",
  unknownUser: "Unknown user",
  linkToMessage: "Link to this message",
  attachmentUnavailable: "Attachment unavailable",
  attachFile: "Attach a file",
  composerMore: "Add a file or a poll",
  attachmentsLabel: "Attachments",
  pendingAttachmentsLabel: "Files to send",
  uploadingFile: "Uploading {name}",
  uploadFailed: "Upload failed",
  removeAttachment: "Remove {name}",
  imageAlt: "Image: {name}",
  openImage: "Open image",
  viewAllImages: "View all {count} images",
  moreImages: "+{count}",
  gallery,
  playVideo: "Play {title}",
  revealSpoiler: "Spoiler, activate to reveal",
  poll,
  paletteLabel: "Colour palette",
  themeModeLabel: "Theme",
  themeModes,
  cannotSendHere: "You can't send messages here.",
  channelActions,
  removeReactor: "Remove {name}'s reaction",
  removeSentAttachment: "Remove attachment {name}",
  notifications,
  search,
  folders,
  pins,
  federation,
  deploymentPermissionNames,
  permissionGroups,
  permissionNames,
  communitySettings,
  roles,
  members,
  access,
  palettes,
  syncStatus,
} as const;

/** A catalogue: the shape of `en`, with any text in place of its strings. */
type Catalogue<T> = { readonly [K in keyof T]: T[K] extends string ? string : Catalogue<T[K]> };

export type Messages = Catalogue<typeof en>;

export function format(template: string, values: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) => values[key] ?? match);
}
