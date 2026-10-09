import { DESKTOP_NOTIFICATIONS, type VoiceRing } from "@aspen/protocol";
import { PhoneIcon, PhoneXIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useContext, useEffect, useState } from "react";
import { closeRingNotifications, postRingNotification } from "./ringNotifications";
import { Button, Dialog, Modal, ModalOverlay } from "react-aria-components";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, type Source } from "@/api/everywhere";
import { useChannel, useMute, useUser } from "@/api/hooks";
import { HomeSyncContext } from "@/api/syncContext";
import { detectShell } from "@/config";
import { primaryButtonClass } from "@/features/auth/styles";
import { Avatar } from "@/features/communities/Avatar";
import { useDmTitle } from "@/features/dms/useDmTitle";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useNow } from "@/features/layout/useNow";
import { channelLink } from "@/features/messages/links";
import { loopSound } from "@/features/notifications/sounds";
import { notificationOutputDevice } from "@/features/settings/audioDevices";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

interface Ringing {
  readonly ring: VoiceRing;
  readonly source: Source;
}

/**
 * Rings the user for a DM call they are not in, on any deployment they use: the first ring
 * still in force shows as `IncomingCall`. A ring ends when they join or decline, when its
 * call ends, or at its `until`, which the clock here decides, as every device does.
 */
export function IncomingCalls() {
  const rings = useEverywhere<readonly Ringing[]>(["rings"], (sources) =>
    sources.flatMap((source) => source.sync.store.myRings().map((ring) => ({ ring, source }))),
  );
  const now = useNow(500, rings.length > 0);
  const current = rings.find(({ ring }) => Date.parse(ring.until) > now);
  const key = current === undefined ? null : ringKey(current);
  // A ring that stopped showing (answered, declined, over, or run out) takes its notification.
  useEffect(() => {
    closeRingNotifications(key);
  }, [key]);
  if (current === undefined || key === null) {
    return null;
  }
  return (
    <SourceScope source={current.source}>
      <IncomingCall key={key} ringId={key} ring={current.ring} source={current.source} />
    </SourceScope>
  );
}

function ringKey({ ring, source }: Ringing): string {
  return `ring/${source.domain ?? ""}/${ring.session}`;
}

/**
 * A call ringing the user: who is calling and from where, with Accept, which opens the DM and
 * joins its call, and Decline, which Escape does too. It has no close button: answering is the
 * way out. While it
 * shows, the ringtone plays and, if the app is not focused, the system notifies, unless the
 * user muted the DM, which rings silently.
 */
function IncomingCall({
  ringId,
  ring,
  source,
}: {
  ringId: string;
  ring: VoiceRing;
  source: Source;
}) {
  const m = useMessages();
  const home = useContext(HomeSyncContext);
  const navigate = useNavigate();
  const channel = useChannel(ring.channel);
  const caller = useUser(ring.caller);
  const muted = useMute(ring.channel) !== undefined;
  const [answered, setAnswered] = useState(false);
  const callerName = caller === undefined ? m.unknownUser : displayNameOf(caller);
  const heading = format(m.voice.incomingCall, { name: callerName });

  useEffect(() => {
    if (muted || answered || home === null) {
      return;
    }
    return loopSound("ringtone", notificationOutputDevice(home.preferences));
  }, [muted, answered, home]);

  // Posted once the caller is known, so its title never changes, and once per ring whatever
  // runs this again (`postRingNotification`); the ring's end closes it.
  const known = caller !== undefined;
  useEffect(() => {
    if (
      !known ||
      muted ||
      home === null ||
      document.hasFocus() ||
      detectShell() === "mobile" ||
      !home.preferences.get(DESKTOP_NOTIFICATIONS) ||
      typeof Notification === "undefined" ||
      Notification.permission !== "granted"
    ) {
      return;
    }
    postRingNotification(
      ringId,
      heading,
      { body: m.voice.incomingCallBody, requireInteraction: true },
      () => {
        window.focus();
      },
    );
  }, [known, muted, home, heading, m, ringId]);

  if (answered || channel === undefined) {
    return null;
  }
  const decline = () => {
    setAnswered(true);
    void source.sync.declineCall(ring.channel).catch(() => undefined);
  };
  const accept = () => {
    setAnswered(true);
    void navigate(channelLink({ domain: source.domain, community: null }, ring.channel));
    void source.sync.voice.join(ring.channel).catch(() => undefined);
  };
  return (
    // A click beside it must not decline a call; Escape still does.
    <ModalOverlay
      isOpen
      onOpenChange={(open) => {
        if (!open) {
          decline();
        }
      }}
      className={overlayClass}
    >
      <Modal className={modalClass + " max-w-xs"}>
        <Dialog role="alertdialog" className={dialogClass + " items-center text-center"}>
          <DialogHeading closeButton={false}>{heading}</DialogHeading>
          <span className="rounded-full motion-safe:animate-pulse">
            <Avatar name={callerName} iconId={caller?.icon} size="lg" />
          </span>
          <Place channel={channel} />
          <div className="flex w-full gap-2">
            <Button
              onPress={decline}
              className={dangerButtonClass + " flex flex-1 items-center justify-center gap-1.5"}
            >
              <PhoneXIcon size={18} aria-hidden="true" />
              {m.voice.decline}
            </Button>
            <Button
              autoFocus
              onPress={accept}
              className={primaryButtonClass + " flex flex-1 items-center justify-center gap-1.5"}
            >
              <PhoneIcon size={18} aria-hidden="true" />
              {m.voice.accept}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** Where the call is coming from: the DM's people, or a group DM's. */
function Place({ channel }: { channel: NonNullable<ReturnType<typeof useChannel>> }) {
  const m = useMessages();
  const title = useDmTitle(channel);
  return (
    <p className="text-sm text-ink-muted">
      {channel.ty === "groupDm"
        ? format(m.voice.incomingGroupCall, { group: title })
        : m.voice.incomingDirectCall}
    </p>
  );
}
