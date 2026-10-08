import type { Community } from "@aspen/protocol";
import { CaretRightIcon } from "@phosphor-icons/react";
import { Button, Disclosure, DisclosurePanel, Heading } from "react-aria-components";
import { useEverywhere } from "@/api/everywhere";
import { usePreference, useSync } from "@/api/hooks";
import { ACTIVITY_HIDDEN, hiddenKey, type FeedPart } from "@/features/activity/filter";
import { CompactCheckbox } from "@/features/layout/choices";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { useMessages } from "@/i18n/context";

/**
 * What the activity feed shows, chosen in its rail: each deployment the reader uses, and within
 * it each of their communities and their DMs. What is left out is remembered on this device
 * (`ACTIVITY_HIDDEN`). Folded away on a one-pane screen, where the rail runs above the feed.
 */
export function ActivityFilters() {
  const m = useMessages();
  const sync = useSync();
  const onePane = useOnePane();
  const hidden = new Set(usePreference(ACTIVITY_HIDDEN));
  const deployments = useEverywhere(["communities"], (sources) =>
    sources.map((source) => ({
      domain: source.domain,
      communities: source.sync.store.communities(),
    })),
  );
  const set = (part: FeedPart, shown: boolean) => {
    const next = new Set(hidden);
    if (shown) {
      next.delete(hiddenKey(part));
    } else {
      next.add(hiddenKey(part));
    }
    void sync.preferences.set(ACTIVITY_HIDDEN, [...next]);
  };
  const isShown = (part: FeedPart) => !hidden.has(hiddenKey(part));
  return (
    <Disclosure defaultExpanded={!onePane} className="group flex min-h-0 flex-col">
      <Heading level={2} className="px-1 pt-2">
        <Button
          slot="trigger"
          className="flex w-full items-center gap-1 rounded px-1 py-1 text-xs font-semibold tracking-wide text-ink-muted uppercase outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <CaretRightIcon
            size={12}
            aria-hidden="true"
            className="transition-transform group-data-[expanded]:rotate-90 rtl:-scale-x-100"
          />
          {m.activity.show}
        </Button>
      </Heading>
      <DisclosurePanel className="min-h-0 overflow-y-auto">
        <div className="flex flex-col gap-2 py-1">
          {deployments.map(({ domain, communities }) => (
            <DeploymentChoices
              key={domain ?? ""}
              domain={domain}
              communities={communities}
              isShown={isShown}
              set={set}
            />
          ))}
        </div>
      </DisclosurePanel>
    </Disclosure>
  );
}

function DeploymentChoices({
  domain,
  communities,
  isShown,
  set,
}: {
  domain: string | null;
  communities: readonly Community[];
  isShown: (part: FeedPart) => boolean;
  set: (part: FeedPart, shown: boolean) => void;
}) {
  const m = useMessages();
  const deployment: FeedPart = { kind: "deployment", domain };
  const shown = isShown(deployment);
  return (
    <fieldset className="flex flex-col">
      <legend className="sr-only">{domain ?? m.activity.thisServer}</legend>
      <CompactCheckbox
        isSelected={shown}
        onChange={(on) => {
          set(deployment, on);
        }}
      >
        <span className="truncate font-medium">{domain ?? m.activity.thisServer}</span>
      </CompactCheckbox>
      <div className="flex flex-col ps-5">
        <CompactCheckbox
          isSelected={isShown({ kind: "dms", domain })}
          isDisabled={!shown}
          onChange={(on) => {
            set({ kind: "dms", domain }, on);
          }}
        >
          <span className="truncate">{m.activity.dms}</span>
        </CompactCheckbox>
        {communities.map((community) => (
          <CompactCheckbox
            key={community.id}
            isSelected={isShown({ kind: "community", domain, community: community.id })}
            isDisabled={!shown}
            onChange={(on) => {
              set({ kind: "community", domain, community: community.id }, on);
            }}
          >
            <span className="truncate">{community.name}</span>
          </CompactCheckbox>
        ))}
      </div>
    </fieldset>
  );
}
