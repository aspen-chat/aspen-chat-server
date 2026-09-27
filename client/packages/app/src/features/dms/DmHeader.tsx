import type { Channel } from "@aspen/protocol";
import {
  ArrowLeftIcon,
  AtIcon,
  SignOutIcon,
  UserPlusIcon,
  UsersThreeIcon,
} from "@phosphor-icons/react";
import { Link, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { Button, Dialog, DialogTrigger, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { PeoplePicker } from "@/features/dms/PeoplePicker";
import { useDmTitle } from "@/features/dms/useDmTitle";
import {
  dialogClass,
  headingClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";

/** The most people a group DM holds, the caller included; the server's `MAX_RECIPIENTS`. */
export const MAX_DM_PEOPLE = 10;

const iconButtonClass =
  "rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The bar above a DM: a way back to the DM list on small screens, and the other people's
 * names. A group DM also offers to add people and to leave.
 */
export function DmHeader({ channel }: { channel: Channel }) {
  const m = useMessages();
  const sync = useSync();
  const title = useDmTitle(channel);
  const group = channel.ty === "groupDm";
  return (
    <header className="flex items-center gap-2 border-b border-line px-4 py-3">
      <Link
        to="/dms"
        aria-label={m.dms.back}
        className="rounded-md p-1 text-ink-muted outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 md:hidden"
      >
        <ArrowLeftIcon size={18} aria-hidden="true" />
      </Link>
      <h2 className="flex min-w-0 flex-1 items-center gap-1.5 truncate font-semibold">
        <span aria-hidden="true" className="text-ink-faint">
          {group ? <UsersThreeIcon size={16} /> : <AtIcon size={16} />}
        </span>
        {title}
      </h2>
      {group && (
        <>
          <PeoplePicker
            trigger={
              <Tooltip text={m.dms.addPeople}>
                <Button aria-label={m.dms.addPeople} className={iconButtonClass}>
                  <UserPlusIcon size={20} aria-hidden="true" />
                </Button>
              </Tooltip>
            }
            heading={m.dms.addPeople}
            confirmLabel={m.dms.add}
            pendingLabel={m.dms.adding}
            exclude={channel.recipients}
            max={MAX_DM_PEOPLE - channel.recipients.length}
            onConfirm={async (ids) => {
              for (const id of ids) {
                await sync.addDmRecipient(channel.id, id);
              }
            }}
          />
          <LeaveGroup channelId={channel.id} />
        </>
      )}
    </header>
  );
}

function LeaveGroup({ channelId }: { channelId: string }) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);
  return (
    <DialogTrigger>
      <Tooltip text={m.dms.leave}>
        <Button aria-label={m.dms.leave} className={iconButtonClass + " hover:text-danger"}>
          <SignOutIcon size={20} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={modalClass}>
          <Dialog role="alertdialog" className={dialogClass}>
            {({ close }) => (
              <>
                <Heading slot="title" className={headingClass}>
                  {m.dms.leaveHeading}
                </Heading>
                <p className="text-sm text-ink-muted">{m.dms.leaveHint}</p>
                {error !== null && (
                  <p role="alert" className="text-sm text-danger">
                    {error}
                  </p>
                )}
                <div className="flex justify-end gap-2">
                  <Button onPress={close} className={secondaryButtonClass}>
                    {m.cancel}
                  </Button>
                  <Button
                    onPress={() => {
                      sync.leaveDm(channelId).then(
                        () => {
                          close();
                          void navigate({ to: "/dms" });
                        },
                        (failure: unknown) => {
                          setError(failure instanceof Error ? failure.message : String(failure));
                        },
                      );
                    }}
                    className="rounded-md bg-danger px-3 py-1.5 text-sm font-medium text-accent-contrast outline-none hover:opacity-90 pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
                  >
                    {m.dms.leave}
                  </Button>
                </div>
              </>
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}
