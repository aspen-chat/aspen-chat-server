import type { Message, PluginInfo } from "@aspen/protocol";
import { usePlugin } from "@/api/hooks";

/** The plugin whose card `message` carries, when it has one the app can draw. */
export function useCardPlugin(message: Message): PluginInfo | undefined {
  const plugin = usePlugin(message.card?.plugin ?? "");
  return message.card == null ? undefined : plugin;
}
