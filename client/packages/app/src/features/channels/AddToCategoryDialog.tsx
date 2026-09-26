import type { Category } from "@aspen/protocol";
import { PlusIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, Dialog, DialogTrigger, Heading, Modal, ModalOverlay } from "react-aria-components";
import { CreateChannelForm } from "@/features/channels/CreateChannelForm";
import { dialogClass, headingClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { OptionButton, StepHeading } from "@/features/layout/steps";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

type Step = "choose" | "text" | "voice";

/**
 * The control at the end of a category's heading and the dialog it opens: pick a text or
 * voice channel, then name it. The new channel is filed under the category without asking.
 */
export function AddToCategoryDialog({ category }: { category: Category }) {
  const m = useMessages();
  const label = format(m.addToCategory, { category: category.name });
  return (
    <DialogTrigger>
      <Tooltip text={label}>
        <Button
          aria-label={label}
          className="rounded p-0.5 text-ink-faint opacity-0 outline-none group-hover:opacity-100 hover:bg-surface-hover hover:text-ink focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <PlusIcon size={14} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => <Steps category={category} close={close} />}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function Steps({ category, close }: { category: Category; close: () => void }) {
  const m = useMessages();
  const [step, setStep] = useState<Step>("choose");
  const back = () => {
    setStep("choose");
  };
  switch (step) {
    case "choose":
      return (
        <>
          <Heading slot="title" className={headingClass}>
            {format(m.newChannelIn, { category: category.name })}
          </Heading>
          <OptionButton
            title={m.addOptions.textChannel}
            hint={m.addOptions.textChannelHint}
            onPress={() => {
              setStep("text");
            }}
          />
          <OptionButton
            title={m.addOptions.voiceChannel}
            hint={m.addOptions.voiceChannelHint}
            onPress={() => {
              setStep("voice");
            }}
          />
        </>
      );
    case "text":
      return (
        <>
          <StepHeading onBack={back}>
            {format(m.newTextChannelIn, { category: category.name })}
          </StepHeading>
          <CreateChannelForm
            communityId={category.community}
            ty="Text"
            parentCategory={category.id}
            onDone={close}
          />
        </>
      );
    case "voice":
      return (
        <>
          <StepHeading onBack={back}>
            {format(m.newVoiceChannelIn, { category: category.name })}
          </StepHeading>
          <CreateChannelForm
            communityId={category.community}
            ty="Voice"
            parentCategory={category.id}
            onDone={close}
          />
        </>
      );
  }
}
