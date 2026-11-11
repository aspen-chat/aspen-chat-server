import type { ChannelType } from "@aspen/protocol";

/** The mark a channel list draws beside a channel's name. */
export type ChannelGlyphKind = "text" | "voice" | "plugin" | "other";

/**
 * Which mark a channel of type `ty` takes in a list of a community's channels. Every type the
 * generated `ChannelType` has is named, so a type it gains does not compile until its mark is
 * chosen. `other` is for the types no such list holds, and for one a newer deployment sends that
 * this build's types lack, which must not pass for a voice channel.
 */
export function channelGlyphKind(ty: ChannelType): ChannelGlyphKind {
  switch (ty) {
    case "text":
      return "text";
    case "voice":
      return "voice";
    case "plugin":
      return "plugin";
    case "thread":
    case "dm":
    case "groupDm":
      return "other";
    default:
      ty satisfies never;
      return "other";
  }
}
