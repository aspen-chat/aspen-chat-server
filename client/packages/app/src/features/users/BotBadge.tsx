import { useMessages } from "@/i18n/context";

/** The mark beside a bot's name, wherever it is named: messages, its card, member lists. */
export function BotBadge() {
  const m = useMessages();
  return (
    <span className="shrink-0 self-center rounded bg-accent-soft px-1 py-px text-[10px] leading-none font-semibold tracking-wide text-accent-strong uppercase">
      {m.bots.badge}
    </span>
  );
}
