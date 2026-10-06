import type { MessageAnnotation, PluginInfo, Severity, UserAnnotation } from "@aspen/protocol";
import { pluginText } from "@aspen/protocol";
import { ArrowSquareOutIcon, InfoIcon, NoteIcon, WarningIcon } from "@phosphor-icons/react";
import { Button, Dialog, DialogTrigger, Popover } from "react-aria-components";
import { useAnnotations, usePlugin, usePlugins, useUserAnnotations } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { webPageUrl } from "@/features/layout/safeUrl";

/** How each severity is drawn: its colours and its icon. */
const SEVERITY: Record<Severity, { className: string; Icon: typeof InfoIcon }> = {
  info: {
    className: "border-line bg-surface-sunken text-ink-muted",
    Icon: InfoIcon,
  },
  notice: {
    className: "border-transparent bg-accent-soft text-accent-strong",
    Icon: NoteIcon,
  },
  warning: {
    className: "border-transparent bg-danger-soft text-danger",
    Icon: WarningIcon,
  },
};

/**
 * One plugin's note, as a chip naming what it says; pressing it tells which plugin said it,
 * with its detail and link, so nothing depends on hover.
 */
function AnnotationChip({
  annotation,
  plugin,
}: {
  annotation: MessageAnnotation | UserAnnotation;
  plugin: PluginInfo;
}) {
  const m = useMessages();
  const { className, Icon } = SEVERITY[annotation.severity];
  const label = pluginText(plugin, annotation.label);
  const link = webPageUrl(annotation.link);
  return (
    <li>
      <DialogTrigger>
        <Button
          className={
            "flex max-w-full items-center gap-1 rounded-full border px-2 py-0.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-accent/50 " +
            className
          }
        >
          <Icon size={12} aria-hidden="true" className="shrink-0" />
          <span className="truncate">{label}</span>
        </Button>
        <Popover className="max-w-xs rounded-md border border-line bg-surface-raised p-3 text-sm shadow-lg">
          <Dialog className="flex flex-col gap-1 outline-none" aria-label={label}>
            <p className="font-medium break-words">{label}</p>
            {annotation.detail != null && (
              <p className="break-words text-ink-muted">{pluginText(plugin, annotation.detail)}</p>
            )}
            <p className="text-xs text-ink-faint">
              {format(m.plugins.noteFrom, { plugin: plugin.name })}
            </p>
            {link !== undefined && (
              <a
                href={link}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-1 text-xs text-accent hover:underline"
              >
                {m.plugins.noteLink}
                <ArrowSquareOutIcon size={12} aria-hidden="true" />
              </a>
            )}
          </Dialog>
        </Popover>
      </DialogTrigger>
    </li>
  );
}

/** The notes of `annotations`, each drawn from its plugin's catalogue. */
function AnnotationList({
  annotations,
}: {
  annotations: readonly (MessageAnnotation | UserAnnotation)[];
}) {
  const m = useMessages();
  const plugins = usePlugins();
  const shown = annotations.flatMap((annotation) => {
    const plugin = plugins.find((p) => p.id === annotation.plugin);
    return plugin === undefined ? [] : [{ annotation, plugin }];
  });
  if (shown.length === 0) {
    return null;
  }
  return (
    <ul aria-label={m.plugins.notesLabel} className="mt-1 flex flex-wrap gap-1">
      {shown.map(({ annotation, plugin }) => (
        <AnnotationChip key={annotation.id} annotation={annotation} plugin={plugin} />
      ))}
    </ul>
  );
}

/** What the deployment's plugins say about a message, beneath it. */
export function MessageAnnotations({ messageId }: { messageId: string }) {
  const annotations = useAnnotations(messageId);
  return annotations.length === 0 ? null : <AnnotationList annotations={annotations} />;
}

/** What the deployment's plugins say about a person, on their card. */
export function UserAnnotations({ userId }: { userId: string }) {
  const annotations = useUserAnnotations(userId);
  return annotations === undefined || annotations.length === 0 ? null : (
    <AnnotationList annotations={annotations} />
  );
}

/**
 * That plugins changed a message's text before it was sent, naming them, beside its text as
 * an edit is marked.
 */
export function AlteredBy({ pluginIds }: { pluginIds: readonly string[] }) {
  const m = useMessages();
  const plugins = usePlugins();
  if (pluginIds.length === 0) {
    return null;
  }
  const names = pluginIds.map((id) => plugins.find((p) => p.id === id)?.name ?? id);
  return (
    <span className="text-xs text-ink-faint" title={m.plugins.alteredByHint}>
      {format(m.plugins.alteredBy, { plugins: names.join(m.plugins.listJoin) })}
    </span>
  );
}

/** The plugin an account belongs to, on its card, for a plugin's own account. */
export function PluginAccount({ pluginId }: { pluginId: string }) {
  const m = useMessages();
  const plugin = usePlugin(pluginId);
  return (
    <p className="text-xs text-ink-muted">
      {format(m.plugins.accountOf, { plugin: plugin?.name ?? pluginId })}
    </p>
  );
}

/**
 * That plugins of this deployment read its DMs, naming them, above a DM, so its people know
 * as the operator who granted it does. Nothing while none does.
 */
export function DmPluginNotice() {
  const m = useMessages();
  const readers = usePlugins().filter((p) => p.dms);
  if (readers.length === 0) {
    return null;
  }
  return (
    <p
      role="note"
      className="flex items-center gap-1.5 border-b border-line bg-surface-sunken px-4 py-1.5 text-xs text-ink-muted"
    >
      <InfoIcon size={14} aria-hidden="true" className="shrink-0" />
      <span>
        {format(m.plugins.dmNotice, {
          plugins: readers.map((p) => p.name).join(m.plugins.listJoin),
        })}{" "}
        <span className="text-ink-faint">{m.plugins.dmNoticeHint}</span>
      </span>
    </p>
  );
}
