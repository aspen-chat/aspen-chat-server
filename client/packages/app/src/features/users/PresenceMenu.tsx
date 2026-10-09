import type { PresenceOverride, User, UserOnlineStatus } from "@aspen/protocol";
import { CaretRightIcon, CheckIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Menu,
  MenuItem,
  Popover,
  SubmenuTrigger,
} from "react-aria-components";
import { useSources } from "@/api/everywhere";
import { useChosenPresence } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { UNTIL } from "@/features/channels/muteEnd";
import { Avatar } from "@/features/communities/Avatar";
import { toast } from "@/features/layout/toast";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { PresenceMark, StatusDot } from "@/features/users/PresenceMark";
import { knownStatus } from "@/features/users/presenceStatus";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format, type Messages } from "@/i18n/messages";

/** How long a chosen status may last, in the menu's order; `null` lasts until it is changed. */
const DURATIONS: readonly {
  key: keyof Messages["presence"]["durations"];
  seconds: number | null;
}[] = [
  { key: "forever", seconds: null },
  { key: "fifteenMinutes", seconds: 15 * 60 },
  { key: "hour", seconds: 60 * 60 },
  { key: "threeHours", seconds: 3 * 60 * 60 },
  { key: "eightHours", seconds: 8 * 60 * 60 },
  { key: "day", seconds: 24 * 60 * 60 },
  { key: "threeDays", seconds: 3 * 24 * 60 * 60 },
];

/** The statuses the user may choose in place of what their connections say, in the menu's order. */
const CHOICES: readonly PresenceOverride[] = ["away", "doNotDisturb", "invisible"];

const popoverClass = "w-72 rounded-md border border-line bg-surface-raised p-1 shadow-lg";
const itemClass =
  "flex cursor-default items-center gap-2 rounded px-2 py-1.5 text-sm outline-none " +
  "focus:bg-surface-hover";
/** An item that opens a submenu stays lit while it is open. */
const parentClass = itemClass + " open:bg-surface-hover";

/**
 * The user's picture, with their status over it, and their name and custom status, in the user
 * bar: pressed, it opens the menu to choose what to show of their presence. Online lets their
 * connections say it again; away, do not disturb, and invisible each open how long to keep it,
 * from a quarter of an hour to three days, or until they change it. The choice is set on every
 * deployment they use, since it is theirs, not one deployment's. `groundClassName` is the
 * background the bar sits on, which the status's disc takes.
 */
export function PresenceMenu({ me, groundClassName }: { me: User; groundClassName: string }) {
  const m = useMessages();
  const sources = useSources();
  const chosen = useChosenPresence();
  const until = useDateFormat(UNTIL);
  const onePane = useOnePane();
  const status = knownStatus(me.onlineStatus);
  const name = displayNameOf(me);
  const choose = (presenceOverride: PresenceOverride | null, seconds: number | null) => {
    void Promise.all(
      sources.map((source) =>
        source.sync.setChosenPresence(presenceOverride, seconds).then(
          () => null,
          (error: unknown) => ({ domain: source.domain, problem: problemText(error) }),
        ),
      ),
    ).then((failures) => {
      for (const failure of failures) {
        if (failure !== null) {
          toast(
            failure.domain === null
              ? format(m.presence.failed, { problem: failure.problem })
              : format(m.presence.failedOn, {
                  deployment: failure.domain,
                  problem: failure.problem,
                }),
          );
        }
      }
    });
  };
  const ends =
    chosen === null
      ? null
      : chosen.until === null
        ? m.presence.forGood
        : format(m.presence.until, { time: until.format(new Date(chosen.until)) });
  return (
    <DialogTrigger>
      <Button
        aria-label={format(m.presence.change, { status: m.status[status] })}
        className="flex min-w-0 flex-1 items-center gap-3 rounded-md p-1 text-start outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        <span className="relative shrink-0">
          <Avatar name={name} iconId={me.icon} size="lg" />
          <StatusDot
            status={status}
            label={m.status[status]}
            large
            groundClassName={groundClassName}
          />
        </span>
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-base font-medium">{name}</span>
          {me.status != null && (
            <span className="truncate text-sm text-ink-muted">{statusLine(me.status)}</span>
          )}
        </span>
      </Button>
      <Popover placement="top start" className={popoverClass}>
        <Dialog aria-label={m.presence.menu} className="outline-none">
          {({ close }) => (
            <Menu
              aria-label={m.presence.menu}
              className="outline-none"
              onAction={(key) => {
                if (key === "online") {
                  close();
                  choose(null, null);
                }
              }}
            >
              <MenuItem id="online" textValue={m.status.online} className={itemClass}>
                <Choice
                  status="online"
                  title={m.status.online}
                  detail={m.presence.descriptions.online}
                  chosen={chosen === null}
                />
              </MenuItem>
              {CHOICES.map((choice) => {
                const isChosen = chosen?.presenceOverride === choice;
                return (
                  <SubmenuTrigger key={choice}>
                    <MenuItem id={choice} textValue={m.status[choice]} className={parentClass}>
                      <Choice
                        status={choice}
                        title={m.status[choice]}
                        detail={isChosen && ends !== null ? ends : m.presence.descriptions[choice]}
                        chosen={isChosen}
                      >
                        <CaretRightIcon
                          size={14}
                          aria-hidden="true"
                          className="shrink-0 text-ink-faint rtl:-scale-x-100"
                        />
                      </Choice>
                    </MenuItem>
                    {/* On a phone a submenu opens over the menu rather than off its side. */}
                    <Popover className={popoverClass} placement={onePane ? "top end" : "end top"}>
                      <Menu
                        aria-label={m.status[choice]}
                        className="outline-none"
                        onAction={(key) => {
                          const duration = DURATIONS.find((d) => d.key === key);
                          if (duration !== undefined) {
                            close();
                            choose(choice, duration.seconds);
                          }
                        }}
                      >
                        {DURATIONS.map((d) => (
                          <MenuItem key={d.key} id={d.key} className={itemClass}>
                            {m.presence.durations[d.key]}
                          </MenuItem>
                        ))}
                      </Menu>
                    </Popover>
                  </SubmenuTrigger>
                );
              })}
            </Menu>
          )}
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

/**
 * One status in the menu: its mark, its name, and a line under it saying what it does, or,
 * for the one chosen, until when; the one in force is ticked.
 */
function Choice({
  status,
  title,
  detail,
  chosen,
  children,
}: {
  status: UserOnlineStatus;
  title: string;
  detail: string;
  chosen: boolean;
  children?: ReactNode;
}) {
  return (
    <>
      <PresenceMark status={status} className="h-3 w-3" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={chosen ? "font-medium" : ""}>{title}</span>
        <span className="text-xs text-ink-muted">{detail}</span>
      </span>
      {chosen && <CheckIcon size={14} aria-hidden="true" className="shrink-0 text-accent" />}
      {children}
    </>
  );
}
