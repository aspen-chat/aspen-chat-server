import { useMessages } from "@/i18n/context";

const badgeClass =
  "shrink-0 self-center rounded bg-accent-soft px-1 py-px text-[10px] leading-none font-semibold tracking-wide text-accent-strong uppercase";

/** The mark beside a bot's name, wherever it is named: messages, its card, member lists. */
export function BotBadge() {
  const m = useMessages();
  return <span className={badgeClass}>{m.bots.badge}</span>;
}

/**
 * The mark beside the system account's name, wherever it is named: the deployment's own
 * account, which sends notices.
 */
export function SystemBadge() {
  const m = useMessages();
  return <span className={badgeClass}>{m.system.badge}</span>;
}
