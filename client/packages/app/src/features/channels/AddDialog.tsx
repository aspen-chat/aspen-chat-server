import type { Community } from "@aspen/protocol";
import { useState } from "react";
import { Button, Dialog, DialogTrigger, Heading, Modal, ModalOverlay } from "react-aria-components";
import { CreateCategoryForm } from "@/features/channels/CreateCategoryForm";
import { CreateChannelForm } from "@/features/channels/CreateChannelForm";
import {
  dialogClass,
  headingClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { InviteManager } from "@/features/invites/InviteDialog";
import { OptionButton, StepHeading } from "@/features/layout/steps";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

type Step = "choose" | "user" | "text" | "voice" | "category";

/**
 * The sidebar's "Add new…" button: pick what to add to the community, then fill it in. A user
 * is added by inviting them, so that choice opens the invite manager.
 */
export function AddDialog({ community }: { community: Community }) {
  const m = useMessages();
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

function Steps({ community, close }: { community: Community; close: () => void }) {
  const m = useMessages();
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
          <Heading slot="title" className={headingClass}>
            {m.addNew}
          </Heading>
          <OptionButton
            title={m.addOptions.user}
            hint={m.addOptions.userHint}
            onPress={choose("user")}
          />
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
          <OptionButton
            title={m.addOptions.category}
            hint={m.addOptions.categoryHint}
            onPress={choose("category")}
          />
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
