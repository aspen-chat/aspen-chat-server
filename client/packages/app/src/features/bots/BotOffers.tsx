import type { BotTransfer } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useMe, useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { primaryButtonClass } from "@/features/auth/styles";
import { TokenReveal } from "@/features/bots/BotsDialog";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

/**
 * Bots others are offering the user, read when Settings' Developer section opens, whether or
 * not developer mode is on: each with who offers it and until when, to accept or decline.
 * Accepting makes the bot theirs and shows its new token, this once. Nothing shows while no
 * offer stands.
 */
export function BotOffers() {
  const m = useMessages();
  const sync = useSync();
  const me = useMe();
  const date = useDateFormat({ dateStyle: "medium", timeStyle: "short" });
  const [offers, setOffers] = useState<readonly BotTransfer[]>([]);
  const [accepted, setAccepted] = useState<{ name: string; token: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    sync.loadBotTransfers().then(
      (all) => {
        if (live) {
          setOffers(all.filter((offer) => offer.to.id === me?.id));
        }
      },
      (e: unknown) => {
        if (live) {
          setError(problemText(e));
        }
      },
    );
    return () => {
      live = false;
    };
  }, [sync, me?.id]);
  const settle = (offer: BotTransfer, action: Promise<unknown>) => {
    setError(null);
    action.then(
      () => {
        setOffers((all) => all.filter((other) => other.bot.id !== offer.bot.id));
      },
      (e: unknown) => {
        setError(problemText(e));
      },
    );
  };
  if (offers.length === 0 && accepted === null && error === null) {
    return null;
  }
  return (
    <section className="flex flex-col gap-2">
      <h4 className="text-sm font-semibold">{m.bots.offersHeading}</h4>
      {accepted !== null && (
        <>
          <p className="text-sm">{format(m.bots.accepted, { name: accepted.name })}</p>
          <TokenReveal name={accepted.name} token={accepted.token} />
        </>
      )}
      <ul className="flex flex-col gap-2">
        {offers.map((offer) => (
          <li key={offer.bot.id} className="flex flex-col gap-2 text-sm">
            <span>
              {format(m.bots.offerFrom, {
                giver: displayNameOf(offer.from),
                bot: displayNameOf(offer.bot),
                date: date.format(new Date(offer.expiresAt)),
              })}
            </span>
            <div className="flex flex-wrap gap-2">
              <Button
                onPress={() => {
                  settle(
                    offer,
                    sync.acceptBotTransfer(offer.bot.id).then(({ bot, token }) => {
                      setAccepted({ name: displayNameOf(bot), token });
                    }),
                  );
                }}
                className={primaryButtonClass}
              >
                {m.bots.accept}
              </Button>
              <Button
                onPress={() => {
                  settle(offer, sync.endBotTransfer(offer.bot.id));
                }}
                className={secondaryButtonClass}
              >
                {m.bots.decline}
              </Button>
            </div>
          </li>
        ))}
      </ul>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
    </section>
  );
}
