import { useState, type ReactNode } from "react";
import { Dialog, DialogTrigger, Modal, ModalOverlay } from "react-aria-components";
import { CreateCommunityForm } from "@/features/communities/CreateCommunityForm";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { JoinForm } from "@/features/invites/JoinForm";
import { OptionButton, StepHeading } from "@/features/layout/steps";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

export type AddCommunityStep = "choose" | "create" | "join";

/**
 * Getting into a community, in two steps: choose between creating one and joining one with an
 * invite, then fill in the name or the invite. `initialStep` skips the choice when the trigger
 * already made it, as the empty state's "Create a community" button does.
 */
export function AddCommunityDialog({
  trigger,
  initialStep = "choose",
}: {
  trigger: ReactNode;
  initialStep?: AddCommunityStep;
}) {
  return (
    <DialogTrigger>
      {trigger}
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => <Steps initialStep={initialStep} close={close} />}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

/** Lives inside the dialog, so the step resets whenever the dialog is reopened. */
function Steps({ initialStep, close }: { initialStep: AddCommunityStep; close: () => void }) {
  const m = useMessages();
  const [step, setStep] = useState<AddCommunityStep>(initialStep);
  const back = () => {
    setStep("choose");
  };

  if (step === "choose") {
    return (
      <>
        <DialogHeading>{m.addCommunity}</DialogHeading>
        <OptionButton
          title={m.createCommunity}
          hint={m.createCommunityHint}
          onPress={() => {
            setStep("create");
          }}
        />
        <OptionButton
          title={m.joinCommunity}
          hint={m.joinCommunityHint}
          onPress={() => {
            setStep("join");
          }}
        />
      </>
    );
  }

  return (
    <>
      <StepHeading onBack={back}>
        {step === "create" ? m.createCommunity : m.joinCommunity}
      </StepHeading>
      {step === "create" ? <CreateCommunityForm onDone={close} /> : <JoinForm onDone={close} />}
    </>
  );
}
