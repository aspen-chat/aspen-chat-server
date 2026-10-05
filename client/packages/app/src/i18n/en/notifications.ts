export const notifications = {
  titleInChannel: "{name} in #{channel} ({community})",
  poll: "Posted a poll",
  attachment: "Sent an attachment",
  newMessage: "New message",
  announced: "{name}: {body}",
  menu: "Notifications",
  notifyMe: "Notify me about",
  default: "Default ({level})",
  levels: {
    all: "All messages",
    tags: "Only tags",
    nothing: "Nothing",
  },
  communityHint: "For channels without a setting of their own. A channel's menu sets its own.",
  settings: "Notifications",
  desktop: "Show system notifications",
  desktopHint:
    "For messages your notification settings ask for, while Aspen is open but you are looking elsewhere.",
  desktopDenied:
    "Your browser blocks notifications from Aspen. Allow them in its site settings, then try again.",
  desktopUnsupported: "This browser cannot show notifications.",
  sounds: "Play a sound",
  soundsHint: "Through the speaker chosen for notification sounds under Audio.",
} as const;
