import type { CommunityPlugin, Permission, PluginInfo } from "@aspen/protocol";
import { PlugIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useAccess, useCommunityPlugins, usePlugins, useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { alertClass, hintClass, primaryButtonClass } from "@/features/auth/styles";
import { planeClass, secondaryButtonClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { toast } from "@/features/layout/toast";
import { SettingsForm } from "@/features/plugins/SettingsForm";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The deployment's plugins as a community's Manage plugins holders see them: each one's name and
 * what it does, whether it runs here, a way to turn on one that runs where it is turned on
 * (granting its account the permissions it asks for, of those the person holds), and its
 * settings here, drawn from what it declares.
 */
export function PluginsPanel({ communityId }: { communityId: string }) {
  const m = useMessages();
  const plugins = usePlugins();
  const used = useCommunityPlugins(communityId);
  if (plugins.length === 0) {
    return <p className={hintClass}>{m.plugins.none}</p>;
  }
  if (used === undefined) {
    return (
      <div aria-busy="true" className="flex flex-col gap-3">
        <LoadingLabel text={m.plugins.loading} />
        <Skeleton className="h-24 w-full rounded-lg" />
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-3">
      <p className={hintClass}>{m.plugins.intro}</p>
      {plugins.map((plugin) => (
        <PluginCard
          key={plugin.id}
          plugin={plugin}
          communityId={communityId}
          use={used.find((u) => u.plugin === plugin.id)}
        />
      ))}
    </div>
  );
}

function PluginCard({
  plugin,
  communityId,
  use,
}: {
  plugin: PluginInfo;
  communityId: string;
  use: CommunityPlugin | undefined;
}) {
  const m = useMessages();
  const sync = useSync();
  const everywhere = plugin.mode === "everywhere";
  const on = everywhere || use?.enabled === true;
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  function turnOff() {
    setPending(true);
    setError(null);
    sync.disableCommunityPlugin(communityId, plugin.id).then(
      () => {
        setPending(false);
        toast(format(m.plugins.turnedOff, { plugin: plugin.name }));
      },
      (e: unknown) => {
        setPending(false);
        setError(problemText(e));
      },
    );
  }

  return (
    <section className={planeClass} aria-labelledby={`plugin-${plugin.id}`}>
      <div className="flex items-start gap-3">
        <PlugIcon size={20} aria-hidden="true" className="mt-0.5 shrink-0 text-ink-muted" />
        <div className="min-w-0 flex-1">
          <h3 id={`plugin-${plugin.id}`} className="flex flex-wrap items-center gap-2 font-medium">
            {plugin.name}
            <span
              className={
                "rounded-full px-2 py-px text-xs " +
                (on ? "bg-accent-soft text-accent-strong" : "bg-surface-sunken text-ink-muted")
              }
            >
              {on ? m.plugins.on : m.plugins.off}
            </span>
          </h3>
          <p className={hintClass}>{plugin.description}</p>
          {everywhere && <p className="text-xs text-ink-faint">{m.plugins.everywhere}</p>}
          {plugin.dms && <p className="text-xs text-ink-faint">{m.plugins.readsDms}</p>}
        </div>
        {on && !everywhere && (
          <Button
            isDisabled={pending}
            onPress={turnOff}
            className={secondaryButtonClass + " shrink-0"}
          >
            {m.plugins.turnOff}
          </Button>
        )}
      </div>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {(!on || (everywhere && plugin.principal != null)) && (
        <TurnOn plugin={plugin} communityId={communityId} everywhere={everywhere} />
      )}
      {on && (
        <div className="flex flex-col gap-2 border-t border-line pt-3">
          <h4 className="text-sm font-semibold text-ink-muted">{m.plugins.settingsHeading}</h4>
          <SettingsForm
            key={JSON.stringify(use?.settings ?? {})}
            plugin={plugin}
            fields={plugin.communitySettings}
            values={(use?.settings ?? {}) as Record<string, unknown>}
            secretsSet={use?.secretsSet ?? []}
            communityId={communityId}
            onSave={async (patch) => {
              await sync.configureCommunityPlugin(communityId, plugin.id, patch);
            }}
          />
        </div>
      )}
    </section>
  );
}

/**
 * Turning a plugin on here, with the permissions its account asks for that the person holds
 * ticked, which they may untick; for a plugin that runs everywhere, bringing its account in.
 */
function TurnOn({
  plugin,
  communityId,
  everywhere,
}: {
  plugin: PluginInfo;
  communityId: string;
  everywhere: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const access = useAccess(communityId);
  const asked = plugin.principalPermissions;
  const [grant, setGrant] = useState<ReadonlySet<Permission>>(
    () => new Set(asked.filter((p) => access?.has(p) === true)),
  );
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function turnOn() {
    setPending(true);
    setError(null);
    sync.enableCommunityPlugin(communityId, plugin.id, {}, [...grant]).then(
      () => {
        setPending(false);
        toast(format(m.plugins.turnedOn, { plugin: plugin.name }));
      },
      (e: unknown) => {
        setPending(false);
        setError(problemText(e));
      },
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {plugin.principal != null && asked.length > 0 && (
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm text-ink-muted">{m.plugins.accountAsks}</legend>
          <div className="grid gap-2 sm:grid-cols-2">
            {asked.map((permission) => (
              <ChoiceCheckbox
                key={permission}
                label={m.permissionNames[permission].name}
                hint={m.permissionNames[permission].hint}
                isSelected={grant.has(permission)}
                isDisabled={access?.has(permission) !== true}
                onChange={(selected) => {
                  const next = new Set(grant);
                  if (selected) {
                    next.add(permission);
                  } else {
                    next.delete(permission);
                  }
                  setGrant(next);
                }}
              />
            ))}
          </div>
          <p className="text-xs text-ink-faint">{m.plugins.accountNeedsAddBots}</p>
        </fieldset>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button isDisabled={pending} onPress={turnOn} className={primaryButtonClass + " self-start"}>
        {everywhere ? m.plugins.bringAccount : m.plugins.turnOn}
      </Button>
    </div>
  );
}
