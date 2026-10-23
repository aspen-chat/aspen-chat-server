import remarkGfm from "remark-gfm";
import type { PluggableList } from "unified";
import { remarkLimits } from "@/features/messages/markdownLimits";
import { remarkBareLinks } from "@/features/messages/remarkBareLinks";
import { remarkCustomEmoji } from "@/features/messages/remarkCustomEmoji";
import { remarkLiteralLinks } from "@/features/messages/remarkLiteralLinks";
import { remarkMentions } from "@/features/messages/remarkMentions";
import { remarkSpoilers } from "@/features/messages/remarkSpoilers";

/**
 * The remark plugins a message body is read with, in order: what `Markdown` renders and what
 * `spoileredLinks` walks, so the two agree on which links a spoiler holds.
 */
export const MESSAGE_REMARK_PLUGINS: PluggableList = [
  remarkGfm,
  remarkLimits,
  remarkLiteralLinks,
  remarkSpoilers,
  remarkMentions,
  remarkCustomEmoji,
  remarkBareLinks,
];
