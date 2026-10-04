/**
 * Drawing what plugins say. A plugin's text is a key of its catalogue (`PluginInfo.messages`,
 * already in the reader's language) and the values its `%{name}` placeholders are filled with,
 * so a client shows any plugin's annotations and settings with no code of the plugin's.
 */

import type { PluginText } from "./generated/events";
import type { PluginInfo } from "./storeTypes";

/**
 * `text` in the reader's language, from `plugin`'s catalogue with its placeholders filled; the
 * key itself when the catalogue lacks it, which a plugin's own checks at install rule out for
 * every key its manifest names.
 */
export function pluginText(plugin: PluginInfo, text: PluginText): string {
  const template = plugin.messages[text.key] ?? text.key;
  return template.replace(/%\{([^}]+)\}/g, (whole, name: string) => text.args?.[name] ?? whole);
}

/** A key of `plugin`'s catalogue with nothing filled in, as setting labels are. */
export function pluginKey(plugin: PluginInfo, key: string): string {
  return pluginText(plugin, { key });
}
