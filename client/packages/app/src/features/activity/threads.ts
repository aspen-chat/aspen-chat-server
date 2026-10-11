/**
 * The feed's messages with each thread's replies folded into one entry: a thread stands once,
 * where its newest reply in the feed stands, and its older replies there are left out, since
 * the entry shows the thread's latest replies itself. `inThread` says whether a deployment's
 * (`key`) channel is a thread. `messages` is newest first, and so is what comes back.
 */
export function foldThreads<T extends { key: string; message: { channelId: string } }>(
  messages: readonly T[],
  inThread: (key: string, channelId: string) => boolean,
): (T & { thread: boolean })[] {
  const folded = new Set<string>();
  const entries: (T & { thread: boolean })[] = [];
  for (const entry of messages) {
    const thread = inThread(entry.key, entry.message.channelId);
    const place = `${entry.key}/${entry.message.channelId}`;
    if (thread && folded.has(place)) {
      continue;
    }
    if (thread) {
      folded.add(place);
    }
    entries.push({ ...entry, thread });
  }
  return entries;
}
