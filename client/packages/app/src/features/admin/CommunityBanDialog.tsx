import type { AdminCommunityEntry, CommunityReach } from "@aspen/protocol";
import { useState } from "react";
import {
  Button,
  Dialog,
  Label,
  Modal,
  ModalOverlay,
  RadioButton,
  RadioField,
  RadioGroup,
} from "react-aria-components";
import { useDeploymentCan, useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { alertClass, labelClass } from "@/features/auth/styles";
import { BanFields } from "@/features/community-settings/BanFields";
import { NEW_BAN, banRequest, type BanChoice } from "@/features/community-settings/banChoice";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
} from "@/features/invites/dialog";
import { ChoiceCheckbox, RadioMark, choiceClass } from "@/features/layout/choices";
import { ConfirmModal } from "@/features/layout/ConfirmModal";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { toast } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

type Contents = "hide" | "erase";

/**
 * Takes a community down from the dashboard: it is deleted for everyone, and the moderator
 * chooses, with no choice made for them, whether everything it holds is hidden or erased, and,
 * holding Ban users, whether its members are banned from the server, with a ban's reason, end,
 * and (holding Remove content) deletion window. Pressing on asks the server whom it reaches
 * and confirms with that count, saying it cannot be undone; done, it says so in a toast and
 * closes.
 */
export function CommunityBanDialog({
  community,
  isOpen,
  onOpenChange,
  onBanned,
}: {
  community: AdminCommunityEntry;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  onBanned: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const mayBan = useDeploymentCan("banUsers");
  const mayDelete = useDeploymentCan("removeContent");
  const [contents, setContents] = useState<Contents | null>(null);
  const [banMembers, setBanMembers] = useState(false);
  const [choice, setChoice] = useState<BanChoice>(NEW_BAN);
  const [counting, setCounting] = useState(false);
  const [reach, setReach] = useState<CommunityReach | null>(null);
  const [error, setError] = useState<string | null>(null);
  const banning = mayBan && banMembers;

  function count() {
    if (counting || contents === null) {
      return;
    }
    setCounting(true);
    setError(null);
    sync.admin.communityReach(community.id).then(
      (counted) => {
        setCounting(false);
        setReach(counted);
      },
      (e: unknown) => {
        setCounting(false);
        setError(problemText(e));
      },
    );
  }

  async function takeDown() {
    if (contents === null) {
      return;
    }
    const outcome = await sync.takeDownCommunity(community.id, {
      erase: contents === "erase",
      ...(banning ? { banMembers: banRequest(choice, mayDelete) } : {}),
    });
    toast(
      outcome.bannedMembers > 0
        ? format(m.admin.bannedCommunityAndMembers, {
            community: community.name,
            count: outcome.bannedMembers,
          })
        : format(m.admin.bannedCommunity, { community: community.name }),
    );
    onOpenChange(false);
    onBanned();
  }

  const explanation =
    reach === null || contents === null
      ? ""
      : [
          format(m.admin.confirmBanCommunityMembers, {
            community: community.name,
            members: reach.members,
          }),
          ...(banning && reach.members > 0
            ? [format(m.admin.confirmBanCommunityBans, { banned: reach.bannable })]
            : []),
          contents === "erase" ? m.admin.confirmBanCommunityErase : m.admin.confirmBanCommunityHide,
          m.admin.confirmBanCommunityPermanent,
        ].join(" ");

  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role="alertdialog" className={dialogClass}>
          <DialogHeading>
            {format(m.admin.banCommunityHeading, { community: community.name })}
          </DialogHeading>
          <p className="text-sm text-ink-muted">{m.admin.banCommunityHint}</p>
          <RadioGroup
            value={contents}
            onChange={(value) => {
              setContents(value === "erase" ? "erase" : "hide");
            }}
            isRequired
            className="flex flex-col gap-2"
          >
            <Label className={labelClass}>{m.admin.communityContents}</Label>
            <ContentsChoice
              value="hide"
              label={m.admin.hideContents}
              hint={m.admin.hideContentsHint}
            />
            <ContentsChoice
              value="erase"
              label={m.admin.eraseContents}
              hint={m.admin.eraseContentsHint}
            />
          </RadioGroup>
          {mayBan && (
            <ChoiceCheckbox
              isSelected={banMembers}
              onChange={setBanMembers}
              label={m.admin.banMembers}
              hint={m.admin.banMembersHint}
            />
          )}
          {banning && (
            <BanFields
              value={choice}
              onChange={setChoice}
              mayDelete={mayDelete}
              reasonHint={m.admin.banMembersReasonHint}
              deleteLabel={m.admin.banMembersDeleteLabel}
            />
          )}
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <div className="flex justify-end">
            <Button
              isDisabled={contents === null || counting}
              onPress={count}
              className={dangerButtonClass}
            >
              {counting ? m.admin.banCommunityCounting : m.admin.banCommunityContinue}
            </Button>
          </div>
          <ConfirmModal
            heading={format(m.admin.confirmBanCommunityHeading, { community: community.name })}
            explanation={explanation}
            action={format(m.admin.confirmBanCommunityAction, { community: community.name })}
            run={takeDown}
            isOpen={reach !== null}
            onOpenChange={(open) => {
              if (!open) {
                setReach(null);
              }
            }}
          />
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function ContentsChoice({ value, label, hint }: { value: Contents; label: string; hint: string }) {
  return (
    <RadioField value={value}>
      <RadioButton className={choiceClass}>
        <RadioMark />
        <span className="flex flex-col">
          <span className="font-medium">{label}</span>
          <span className="text-xs text-ink-muted">{hint}</span>
        </span>
      </RadioButton>
    </RadioField>
  );
}
