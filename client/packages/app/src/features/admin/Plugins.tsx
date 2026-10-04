import type { AdminPlugin } from "@aspen/protocol";
import { ArrowDownIcon, ArrowUpIcon, CaretDownIcon } from "@phosphor-icons/react";
import { useCallback, useState } from "react";
import {
  Button,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import { useDeploymentCan, useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { useFigures } from "@/features/admin/format";
import { useAdminRead } from "@/features/admin/useAdminRead";
import {
  alertClass,
  fieldClass,
  hintClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  optionClass,
  planeSurfaceClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { Skeleton } from "@/features/layout/Skeleton";
import { Tooltip } from "@/features/layout/Tooltip";
import { SettingsForm } from "@/features/plugins/SettingsForm";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const MODES = ["optIn", "everywhere"] as const;

/**
 * The installed plugins, in the order they decide messages in: each one's name, what it does,
 * what it was granted, the hosts it may call, what it keeps, and the storage it uses. A holder
 * of Manage plugins turns each on or off, chooses where it runs, orders them, and changes their
 * settings; installing, upgrading, and removing them is the terminal's.
 */
export function PluginsSection() {
  const m = useMessages();
  const sync = useSync();
  const manage = useDeploymentCan("managePlugins");
  const load = useCallback(() => sync.admin.plugins(), [sync]);
  const read = useAdminRead(load);
  const plugins = read.data;
  const [error, setError] = useState<string | null>(null);

  function move(index: number, by: -1 | 1) {
    if (plugins === undefined) {
      return;
    }
    const ids = plugins.map((p) => p.plugin.id);
    const [moved] = ids.splice(index, 1);
    if (moved === undefined) {
      return;
    }
    ids.splice(index + by, 0, moved);
    setError(null);
    sync.admin.orderPlugins(ids).then(read.reload, (e: unknown) => {
      setError(problemText(e));
    });
  }

  return (
    <Section id="admin-plugins" title={m.plugins.adminTab} hint={m.plugins.adminIntro}>
      {read.error !== null && <ReadFailed error={read.error} onRetry={read.reload} />}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {plugins === undefined ? (
        read.error === null && <Skeleton className="h-32 w-full rounded-lg" />
      ) : plugins.length === 0 ? (
        <p className={hintClass}>{m.plugins.adminNone}</p>
      ) : (
        <>
          {plugins.length > 1 && <p className={hintClass}>{m.plugins.order}</p>}
          <ol className="flex flex-col gap-3">
            {plugins.map((plugin, index) => (
              <InstalledPlugin
                key={plugin.plugin.id}
                plugin={plugin}
                manage={manage}
                onChanged={read.reload}
                {...(manage && index > 0
                  ? {
                      onEarlier: () => {
                        move(index, -1);
                      },
                    }
                  : {})}
                {...(manage && index < plugins.length - 1
                  ? {
                      onLater: () => {
                        move(index, 1);
                      },
                    }
                  : {})}
              />
            ))}
          </ol>
        </>
      )}
    </Section>
  );
}

function InstalledPlugin({
  plugin: installed,
  manage,
  onChanged,
  onEarlier,
  onLater,
}: {
  plugin: AdminPlugin;
  manage: boolean;
  onChanged: () => void;
  onEarlier?: () => void;
  onLater?: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const figures = useFigures();
  const { plugin } = installed;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function update(patch: Parameters<typeof sync.admin.updatePlugin>[1]) {
    setPending(true);
    setError(null);
    return sync.admin.updatePlugin(plugin.id, patch).then(
      () => {
        setPending(false);
        onChanged();
      },
      (e: unknown) => {
        setPending(false);
        setError(problemText(e));
        throw e;
      },
    );
  }

  return (
    <li className={planeSurfaceClass + " flex flex-col gap-3"}>
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="flex flex-wrap items-center gap-2 font-medium">
            {plugin.name}
            <span
              className={
                "rounded-full px-2 py-px text-xs " +
                (installed.enabled
                  ? "bg-accent-soft text-accent-strong"
                  : "bg-surface-sunken text-ink-muted")
              }
            >
              {installed.enabled ? m.plugins.on : m.plugins.off}
            </span>
          </h3>
          <p className={hintClass}>{plugin.description}</p>
          <p className="text-xs text-ink-faint">
            {plugin.id} · {format(m.plugins.version, { version: plugin.version })}
            {plugin.author != null && <> · {format(m.plugins.by, { author: plugin.author })}</>}
          </p>
        </div>
        {manage && (
          <div className="flex items-center gap-1">
            {onEarlier !== undefined && (
              <Tooltip text={format(m.plugins.moveUp, { plugin: plugin.name })}>
                <Button
                  aria-label={format(m.plugins.moveUp, { plugin: plugin.name })}
                  onPress={onEarlier}
                  className="tap-target rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
                >
                  <ArrowUpIcon size={16} aria-hidden="true" />
                </Button>
              </Tooltip>
            )}
            {onLater !== undefined && (
              <Tooltip text={format(m.plugins.moveDown, { plugin: plugin.name })}>
                <Button
                  aria-label={format(m.plugins.moveDown, { plugin: plugin.name })}
                  onPress={onLater}
                  className="tap-target rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
                >
                  <ArrowDownIcon size={16} aria-hidden="true" />
                </Button>
              </Tooltip>
            )}
            <Button
              isDisabled={pending}
              onPress={() => {
                void update({ enabled: !installed.enabled }).catch(() => undefined);
              }}
              className={installed.enabled ? secondaryButtonClass : primaryButtonClass}
            >
              {installed.enabled ? m.plugins.turnOff : m.plugins.turnOn}
            </Button>
          </div>
        )}
      </div>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[auto_1fr]">
        <dt className="text-ink-muted">{m.plugins.granted}</dt>
        <dd>
          <ul className="flex flex-col">
            {installed.granted.map((p) => (
              <li key={p}>{m.plugins.permissions[p]}</li>
            ))}
          </ul>
        </dd>
        {installed.hosts.length > 0 && (
          <>
            <dt className="text-ink-muted">{m.plugins.hosts}</dt>
            <dd className="break-all">{installed.hosts.join(", ")}</dd>
          </>
        )}
        <dt className="text-ink-muted">{m.plugins.retention}</dt>
        <dd>{installed.retention}</dd>
        {installed.storageQuota != null && (
          <>
            <dt className="text-ink-muted">{m.plugins.storageLabel}</dt>
            <dd>
              {format(m.plugins.storage, {
                used: figures.bytes(installed.storageBytes),
                quota: figures.bytes(installed.storageQuota),
              })}
            </dd>
          </>
        )}
      </dl>
      {manage && (
        <Select
          value={plugin.mode}
          onChange={(key) => {
            const mode = MODES.find((x) => x === key);
            if (mode !== undefined && mode !== plugin.mode) {
              void update({ mode }).catch(() => undefined);
            }
          }}
          isDisabled={pending}
          className={fieldClass + " max-w-sm"}
        >
          <Label className={labelClass}>{m.plugins.mode}</Label>
          <Button className={selectButtonClass + " py-2"}>
            <SelectValue />
            <CaretDownIcon size={14} aria-hidden="true" className="text-ink-muted" />
          </Button>
          <Popover className={selectPopoverClass}>
            <ListBox>
              <ListBoxItem id="optIn" className={optionClass}>
                {m.plugins.modeOptIn}
              </ListBoxItem>
              <ListBoxItem id="everywhere" className={optionClass}>
                {m.plugins.modeEverywhere}
              </ListBoxItem>
            </ListBox>
          </Popover>
        </Select>
      )}
      {manage && installed.settingsFields.length > 0 && (
        <div className="flex flex-col gap-2 border-t border-line pt-3">
          <h4 className="text-sm font-semibold text-ink-muted">{m.plugins.settingsHeading}</h4>
          <SettingsForm
            key={JSON.stringify(installed.settings)}
            plugin={plugin}
            fields={installed.settingsFields}
            values={installed.settings}
            secretsSet={installed.secretsSet}
            onSave={async (settings) => {
              await update({ settings });
            }}
          />
        </div>
      )}
    </li>
  );
}
