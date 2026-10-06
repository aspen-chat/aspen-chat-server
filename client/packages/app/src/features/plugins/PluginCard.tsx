import { ApiProblemError, pluginText, type Message, type PluginInfo } from "@aspen/protocol";
import { useState } from "react";
import { Button } from "react-aria-components";
import { usePlugin, useSync } from "@/api/hooks";
import {
  accentButtonClass,
  dangerButtonClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { PersonName } from "@/features/users/PersonName";
import { useMessages } from "@/i18n/context";
import { useDateFormat, useNumberFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import { webPageUrl } from "@/features/layout/safeUrl";

type Card = NonNullable<Message["card"]>;
type CardField = Card["fields"][number];
type CardButton = Card["buttons"][number];

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "full", timeStyle: "short" };

const BUTTON_CLASS: Record<CardButton["style"], string> = {
  primary: accentButtonClass,
  secondary: secondaryButtonClass,
  danger: dangerButtonClass,
};

/**
 * The card a plugin's account put on its message (`spec/plugins.md`, Cards): a title, fields
 * (times in the reader's zone and language, people by name), and buttons, each of which calls
 * the plugin as the reader, who hears of the change as the message's update. Nothing is drawn
 * for a card of a plugin the app does not know, whose text it has no catalogue for.
 */
export function PluginCard({ message, still }: { message: Message; still: boolean }) {
  const card = message.card;
  const plugin = usePlugin(card?.plugin ?? "");
  if (card == null || plugin === undefined) {
    return null;
  }
  const title = card.title == null ? null : pluginText(plugin, card.title);
  return (
    <section
      aria-label={title ?? plugin.name}
      className="mt-1 flex w-full max-w-lg flex-col gap-2 rounded-md border border-line bg-surface-raised p-3"
    >
      {title !== null && <h3 className="font-medium break-words">{title}</h3>}
      {card.fields.length > 0 && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
          {card.fields.map((field, index) => (
            <Field key={index} plugin={plugin} field={field} />
          ))}
        </dl>
      )}
      {!still && card.buttons.length > 0 && (
        <Buttons messageId={message.id} plugin={plugin} buttons={card.buttons} />
      )}
    </section>
  );
}

function Field({ plugin, field }: { plugin: PluginInfo; field: CardField }) {
  return (
    <>
      <dt className="text-ink-muted">{pluginText(plugin, field.label)}</dt>
      <dd className="min-w-0 break-words">
        <Value plugin={plugin} value={field.value} />
      </dd>
    </>
  );
}

function Value({ plugin, value }: { plugin: PluginInfo; value: CardField["value"] }) {
  const time = useDateFormat(TIME);
  const numbers = useNumberFormat();
  switch (value.type) {
    case "plain":
      return <>{value.text}</>;
    case "time":
      return <time dateTime={value.at}>{time.format(new Date(value.at))}</time>;
    case "count":
      return <>{numbers.format(value.count)}</>;
    case "person":
      return <PersonName id={value.user} />;
    case "link": {
      const href = webPageUrl(value.url);
      if (href === undefined) {
        return <>{pluginText(plugin, value.text)}</>;
      }
      return (
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="text-accent underline underline-offset-2 hover:decoration-2"
        >
          {pluginText(plugin, value.text)}
        </a>
      );
    }
  }
}

/** The card's buttons, each disabled while its press is on its way, and why one failed. */
function Buttons({
  messageId,
  plugin,
  buttons,
}: {
  messageId: string;
  plugin: PluginInfo;
  buttons: readonly CardButton[];
}) {
  const m = useMessages();
  const sync = useSync();
  const [pressing, setPressing] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  return (
    <>
      <div className="flex flex-wrap gap-2">
        {buttons.map((button) => (
          <Button
            key={button.id}
            isDisabled={pressing !== null}
            onPress={() => {
              setPressing(button.id);
              setFailure(null);
              sync
                .pressCardButton(messageId, button.id)
                .then(
                  (status) => {
                    if (status >= 400) {
                      setFailure(format(m.plugins.pressFailed, { plugin: plugin.name }));
                    }
                  },
                  (error: unknown) => {
                    setFailure(
                      error instanceof ApiProblemError
                        ? error.message
                        : format(m.plugins.pressFailed, { plugin: plugin.name }),
                    );
                  },
                )
                .finally(() => {
                  setPressing(null);
                });
            }}
            className={BUTTON_CLASS[button.style]}
          >
            {pluginText(plugin, button.label)}
          </Button>
        ))}
      </div>
      {failure !== null && (
        <p role="alert" className="text-sm text-danger">
          {failure}
        </p>
      )}
    </>
  );
}
