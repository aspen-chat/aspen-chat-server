import type { Community } from "@aspen/protocol";
import { useState } from "react";
import { useCan } from "@/api/hooks";
import { Button, Dialog, DialogTrigger, Modal, ModalOverlay } from "react-aria-components";
import { CreateCategoryForm } from "@/features/channels/CreateCategoryForm";
import { CreateChannelForm } from "@/features/channels/CreateChannelForm";
import {
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { InviteManager } from "@/features/invites/InviteDialog";
import { OptionButton, StepHeading } from "@/features/layout/steps";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

type Step = "choose" | "user" | "text" | "voice" | "category";

/**
 * The sidebar's "Add new…" button: pick what to add to the community, then fill it in. A user
 * is added by inviting them, so that choice opens the invite manager. Only what the caller may
 * add is offered, and without anything to offer there is no button.
 */
export function AddDialog({ community }: { community: Community }) {
  const m = useMessages();
  const offers = useOffers(community.id);
  if (!offers.user && !offers.channel && !offers.category) {
    return null;
  }
  return (
    <DialogTrigger>
      <Button className={secondaryButtonClass + " w-full"}>{m.addNew}</Button>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => <Steps community={community} close={close} />}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

/** What the caller may add to a community. */
function useOffers(communityId: string): { user: boolean; channel: boolean; category: boolean } {
  return {
    user: useCan(communityId, "createInvites"),
    channel: useCan(communityId, "manageChannels"),
    category: useCan(communityId, "manageCategories"),
  };
}

function Steps({ community, close }: { community: Community; close: () => void }) {
  const m = useMessages();
  const offers = useOffers(community.id);
  const [step, setStep] = useState<Step>("choose");
  const back = () => {
    setStep("choose");
  };
  const choose = (next: Step) => () => {
    setStep(next);
  };

  switch (step) {
    case "choose":
      return (
        <>
          <DialogHeading>{m.addNew}</DialogHeading>
          {offers.user && (
            <OptionButton
              title={m.addOptions.user}
              hint={m.addOptions.userHint}
              onPress={choose("user")}
            />
          )}
          {offers.channel && (
            <>
              <OptionButton
                title={m.addOptions.textChannel}
                hint={m.addOptions.textChannelHint}
                onPress={choose("text")}
              />
              <OptionButton
                title={m.addOptions.voiceChannel}
                hint={m.addOptions.voiceChannelHint}
                onPress={choose("voice")}
              />
            </>
          )}
          {offers.category && (
            <OptionButton
              title={m.addOptions.category}
              hint={m.addOptions.categoryHint}
              onPress={choose("category")}
            />
          )}
        </>
      );
    case "user":
      return (
        <>
          <StepHeading onBack={back}>
            {format(m.inviteDialogHeading, { community: community.name })}
          </StepHeading>
          <InviteManager communityId={community.id} />
        </>
      );
    case "text":
      return (
        <>
          <StepHeading onBack={back}>{m.newTextChannel}</StepHeading>
          <CreateChannelForm communityId={community.id} ty="text" onDone={close} />
        </>
      );
    case "voice":
      return (
        <>
          <StepHeading onBack={back}>{m.newVoiceChannel}</StepHeading>
          <CreateChannelForm communityId={community.id} ty="voice" onDone={close} />
        </>
      );
    case "category":
      return (
        <>
          <StepHeading onBack={back}>{m.newCategory}</StepHeading>
          <CreateCategoryForm communityId={community.id} onDone={close} />
        </>
      );
  }
}
