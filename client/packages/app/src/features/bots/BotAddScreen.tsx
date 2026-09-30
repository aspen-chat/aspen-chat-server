import { ApiProblemError, type Permission } from "@aspen/protocol";
import { CaretDownIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";
import {
  Button,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import { useCommunities, useMe, useStore, useSync, useUser } from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import { suggestedPermissions } from "@/features/bots/botLink";
import { Avatar } from "@/features/communities/Avatar";
import { optionClass, selectButtonClass, selectPopoverClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { BotBadge } from "@/features/users/BotBadge";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";

/**
 * What a bot's link opens: the bot, the caller's communities they may add bots to, and the
 * permissions the link suggests, each of which the caller may leave out. Giving any takes
 * Manage roles and Assign roles there, and only permissions the caller holds may be given; the
 * rest are shown and cannot be chosen. A private bot only its owner may add.
 */
export function BotAddScreen({
  botId,
  permissions: param,
}: {
  botId: string;
  permissions: string | undefined;
}) {
  const m = useMessages();
  const sync = useSync();
  const store = useStore();
  const navigate = useNavigate();
  const me = useMe();
  const bot = useUser(botId);
  const suggested = suggestedPermissions(param);
  const places = useCommunities().filter((c) => store.access(c.id)?.has("addBots") === true);
  const [chosenCommunity, setChosenCommunity] = useState<string | null>(null);
  const communityId = chosenCommunity ?? places[0]?.id ?? null;
  const access = communityId === null ? null : store.access(communityId);
  const mayGrant = access?.has("manageRoles") === true && access.has("assignRoles");
  const grantable = (p: Permission) => access !== null && mayGrant && access.has(p);
  const [left, setLeft] = useState<ReadonlySet<Permission>>(new Set());
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (bot !== undefined && !bot.bot) {
    return <Frame>{m.bots.notABot}</Frame>;
  }
  const name = bot === undefined ? m.loading : displayNameOf(bot);
  const mayAdd = bot !== undefined && (bot.botPublic || (me !== null && bot.botOwner === me.id));
  const giving = suggested.filter((p) => grantable(p) && !left.has(p));

  return (
    <Frame>
      <div className="flex items-center gap-3">
        {bot === undefined ? (
          <>
            <LoadingLabel />
            <Skeleton className="h-12 w-12 shrink-0 rounded-full" />
            <Skeleton className="h-6 w-48" />
          </>
        ) : (
          <>
            <Avatar name={name} iconId={bot.icon} size="lg" />
            <div className="flex min-w-0 items-center gap-2">
              <h1 className="truncate text-xl font-semibold">
                {format(m.bots.addHeading, { name })}
              </h1>
              <BotBadge />
            </div>
          </>
        )}
      </div>
      {bot !== undefined && !mayAdd && <p className="text-sm text-danger">{m.bots.privateBot}</p>}
      {places.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.bots.addNoCommunities}</p>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (communityId === null) {
              return;
            }
            setPending(true);
            setError(null);
            sync.addBot(communityId, botId, giving).then(
              () => {
                void navigate({
                  to: "/communities/$communityId",
                  params: { communityId },
                });
              },
              (e: unknown) => {
                setError(e instanceof ApiProblemError ? e.message : String(e));
                setPending(false);
              },
            );
          }}
          className="flex flex-col gap-4"
        >
          <Select
            value={communityId}
            onChange={(key) => {
              if (typeof key === "string") {
                setChosenCommunity(key);
              }
            }}
            className="flex flex-col gap-1"
          >
            <Label className="text-sm font-medium">{m.bots.addCommunity}</Label>
            <Button className={selectButtonClass}>
              <SelectValue className="truncate" />
              <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
            </Button>
            <Popover className={selectPopoverClass}>
              <ListBox items={places}>
                {(community) => (
                  <ListBoxItem id={community.id} textValue={community.name} className={optionClass}>
                    {community.name}
                  </ListBoxItem>
                )}
              </ListBox>
            </Popover>
          </Select>
          {suggested.length > 0 && (
            <fieldset className="flex flex-col gap-2">
              <legend className="text-sm font-medium">{m.bots.addPermissions}</legend>
              <p className="text-xs text-ink-muted">{m.bots.addPermissionsHint}</p>
              <div className="grid gap-2 sm:grid-cols-2">
                {suggested.map((permission) => (
                  <ChoiceCheckbox
                    key={permission}
                    isSelected={grantable(permission) && !left.has(permission)}
                    isDisabled={!grantable(permission)}
                    onChange={(selected) => {
                      const next = new Set(left);
                      if (selected) {
                        next.delete(permission);
                      } else {
                        next.add(permission);
                      }
                      setLeft(next);
                    }}
                    label={m.permissionNames[permission].name}
                    hint={m.permissionNames[permission].hint}
                  />
                ))}
              </div>
            </fieldset>
          )}
          {error !== null && (
            <p role="alert" className="text-sm text-danger">
              {error}
            </p>
          )}
          <Button
            type="submit"
            isDisabled={pending || !mayAdd || communityId === null}
            className={primaryButtonClass + " self-start"}
          >
            {pending ? m.bots.adding : m.bots.add}
          </Button>
        </form>
      )}
    </Frame>
  );
}

function Frame({ children }: { children: ReactNode }) {
  return (
    <main className="min-w-0 flex-1 overflow-y-auto bg-surface">
      <div className="mx-auto flex max-w-2xl flex-col gap-6 px-4 py-6 md:px-6">{children}</div>
    </main>
  );
}
