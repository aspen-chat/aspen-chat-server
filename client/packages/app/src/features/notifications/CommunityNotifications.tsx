import type { NotificationLevel } from "@aspen/protocol";
import { Label, RadioButton, RadioField, RadioGroup, Text } from "react-aria-components";
import { useCommunityNotificationLevel, useSync } from "@/api/hooks";
import { choiceClass, RadioMark } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const LEVELS: readonly NotificationLevel[] = ["all", "tags", "nothing"];

/**
 * How much of a community's channels the user is told of, for those without a setting of their
 * own: its default (tags), every message, tags, or nothing.
 */
export function CommunityNotifications({ communityId }: { communityId: string }) {
  const m = useMessages();
  const sync = useSync();
  const level = useCommunityNotificationLevel(communityId);
  return (
    <RadioGroup
      value={level ?? "default"}
      onChange={(value) => {
        void sync
          .setCommunityNotifications(
            communityId,
            value === "default" ? null : (value as NotificationLevel),
          )
          .catch(() => undefined);
      }}
      className="flex flex-col gap-2"
    >
      <Label className="text-sm font-semibold text-ink-muted">{m.notifications.notifyMe}</Label>
      <Text slot="description" className="text-xs text-ink-muted">
        {m.notifications.communityHint}
      </Text>
      <RadioField value="default">
        <RadioButton className={choiceClass}>
          <RadioMark />
          {format(m.notifications.default, { level: m.notifications.levels.tags })}
        </RadioButton>
      </RadioField>
      {LEVELS.map((choice) => (
        <RadioField key={choice} value={choice}>
          <RadioButton className={choiceClass}>
            <RadioMark />
            {m.notifications.levels[choice]}
          </RadioButton>
        </RadioField>
      ))}
    </RadioGroup>
  );
}
