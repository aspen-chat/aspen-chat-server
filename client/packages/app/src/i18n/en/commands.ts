export const commands = {
  suggestions: "Bots' commands",
  oneSuggestion: "1 command. Enter or Tab picks it; Escape closes.",
  someSuggestions:
    "{count} commands. Up and down arrows choose, Enter or Tab picks; Escape closes.",
  values: "Suggestions for {parameter}",
  oneValue: "1 suggestion for {parameter}. Enter or Tab picks it; Escape closes.",
  someValues:
    "{count} suggestions for {parameter}. Up and down arrows choose, Enter or Tab picks; Escape closes.",
  optional: "optional",
  takes: {
    userId: "A person",
    channelId: "A channel",
    messageId: "A link to a message",
    communityId: "A community",
    roleId: "A role",
    attachmentId: "A file attached to the message",
    deploymentHost: "A deployment's domain",
    react: "One emoji",
    any: "Any text",
    regex: "Text of the form the bot asks for",
  },
  doesNotFit: "{parameter} doesn't take this. Check the command's description for the form.",
  ambiguous:
    "More than one bot answers /{name}: {bots}. Put the one you mean after the name, as in /{name} @{example}.",
  sentTo: "sent {bot} a command",
} as const;
