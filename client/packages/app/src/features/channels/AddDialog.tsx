import type { Community, PluginInfo } from "@aspen/protocol";
import { useState } from "react";
import { useCan, usePlugins } from "@/api/hooks";
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
import { OptionButton } from "@/features/layout/steps";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

type Step =
  | "choose"
  | "user"
  | "text"
  | "voice"
  | "category"
  | { plugin: PluginInfo; kind: PluginInfo["channelTypes"][number] };

/**
 * The sidebar's "Add new…" button: pick what to add to the community, then fill it in. A user
 * is added by inviting them, so that choice opens the invite manager. Kinds of channel the
 * deployment's plugins add are offered beside text and voice; the server refuses one whose plugin
 * does not run in the community, saying so. Only what the caller may add is offered, and without
 * anything to offer there is no button.
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
  const plugins = usePlugins();
  const [step, setStep] = useState<Step>("choose");
  const back = () => {
    setStep("choose");
  };
  const choose = (next: Step) => () => {
    setStep(next);
  };

  if (typeof step === "object") {
    return (
      <>
        <DialogHeading onBack={back}>
          {format(m.plugins.newChannelOfKind, { kind: step.kind.name })}
        </DialogHeading>
        <CreateChannelForm
          communityId={community.id}
          ty="plugin"
          pluginType={step.kind.pluginType}
          onDone={close}
        />
      </>
    );
  }
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
              {plugins.flatMap((plugin) =>
                plugin.channelTypes.map((kind) => (
                  <OptionButton
                    key={kind.pluginType}
                    title={kind.name}
                    hint={format(m.plugins.channelKindHint, { plugin: plugin.name })}
                    onPress={choose({ plugin, kind })}
                  />
                )),
              )}
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
          <DialogHeading onBack={back}>
            {format(m.inviteDialogHeading, { community: community.name })}
          </DialogHeading>
          <InviteManager communityId={community.id} />
        </>
      );
    case "text":
      return (
        <>
          <DialogHeading onBack={back}>{m.newTextChannel}</DialogHeading>
          <CreateChannelForm communityId={community.id} ty="text" onDone={close} />
        </>
      );
    case "voice":
      return (
        <>
          <DialogHeading onBack={back}>{m.newVoiceChannel}</DialogHeading>
          <CreateChannelForm communityId={community.id} ty="voice" onDone={close} />
        </>
      );
    case "category":
      return (
        <>
          <DialogHeading onBack={back}>{m.newCategory}</DialogHeading>
          <CreateCategoryForm communityId={community.id} onDone={close} />
        </>
      );
  }
}
