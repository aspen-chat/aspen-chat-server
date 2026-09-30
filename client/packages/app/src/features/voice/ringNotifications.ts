/**
 * The system notifications posted for calls ringing the user, one per ring, kept here rather
 * than in a component: an effect may run more than once for one ring (React runs effects twice
 * in development, and again when what they read changes), and a notification posted again in
 * quick succession is one a desktop may refuse as spam.
 */
const posted = new Map<string, Notification>();

/** Posts the notification for the ring `key`, unless one was posted for it already. */
export function postRingNotification(
  key: string,
  title: string,
  options: NotificationOptions,
  onClick: () => void,
): void {
  if (posted.has(key)) {
    return;
  }
  const notification = new Notification(title, { ...options, tag: key });
  notification.onclick = onClick;
  posted.set(key, notification);
}

/** Closes every ring's notification but `keep`'s: those rings were answered or ended. */
export function closeRingNotifications(keep: string | null = null): void {
  for (const [key, notification] of posted) {
    if (key !== keep) {
      notification.close();
      posted.delete(key);
    }
  }
}
