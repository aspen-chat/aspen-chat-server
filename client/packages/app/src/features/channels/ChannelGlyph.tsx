import type { Channel } from "@aspen/protocol";
import { CircleDashedIcon, HashIcon, SpeakerHighIcon } from "@phosphor-icons/react";
import { usePluginKind } from "@/api/hooks";
import { channelGlyphKind } from "@/features/channels/channelGlyphKind";
import { PluginGlyph } from "@/features/plugins/PluginChannel";

/** The mark beside a channel's name in a list of a community's channels, by its type. */
export function ChannelGlyph({ channel }: { channel: Channel }) {
  const kind = usePluginKind(channel);
  switch (channelGlyphKind(channel.ty)) {
    case "text":
      return <HashIcon size={16} aria-hidden="true" className="shrink-0 text-ink-faint" />;
    case "voice":
      return <SpeakerHighIcon size={16} aria-hidden="true" className="shrink-0" />;
    case "plugin":
      return <PluginGlyph glyph={kind?.kind.glyph} size={16} className="shrink-0 text-ink-faint" />;
    case "other":
      return <CircleDashedIcon size={16} aria-hidden="true" className="shrink-0 text-ink-faint" />;
  }
}
