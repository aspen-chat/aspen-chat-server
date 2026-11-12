import remarkGfm from "remark-gfm";
import type { PluggableList } from "unified";
import { remarkLimits } from "@/features/messages/markdownLimits";
import { remarkBareLinks } from "@/features/messages/remarkBareLinks";
import { remarkMarkerText } from "@/features/messages/markerText";
import { remarkCustomEmoji } from "@/features/messages/remarkCustomEmoji";
import { remarkLiteralLinks } from "@/features/messages/remarkLiteralLinks";
import { remarkMentions } from "@/features/messages/remarkMentions";
import { remarkSpoilers } from "@/features/messages/remarkSpoilers";

/**
 * The plugins that decide where a message's spoilers are, in order: the first of what a
 * message body is read with, and all `maskSpoilers` reads one with.
 */
export const SPOILER_REMARK_PLUGINS: PluggableList = [
  remarkMarkerText,
  remarkGfm,
  remarkLimits,
  remarkLiteralLinks,
  remarkSpoilers,
];

/**
 * The remark plugins a message body is read with, in order: what `Markdown` renders and what
 * `spoileredLinks` walks, so the two agree on which links a spoiler holds.
 */
export const MESSAGE_REMARK_PLUGINS: PluggableList = [
  ...SPOILER_REMARK_PLUGINS,
  remarkMentions,
  remarkCustomEmoji,
  remarkBareLinks,
];
