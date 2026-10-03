import { ApiProblemError, type ProfileAspect, type ReportCase, type User } from "@aspen/protocol";
import { useState } from "react";
import {
  Button,
  Dialog,
  Label,
  Modal,
  ModalOverlay,
  TextArea,
  TextField,
} from "react-aria-components";
import { useDeploymentCan, useSync } from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { BanFields } from "@/features/community-settings/BanFields";
import { NEW_BAN, banRequest, type BanChoice } from "@/features/community-settings/banChoice";
import { dialogClass, overlayClass, wideModalClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { toast } from "@/features/layout/toast";
import { PROFILE_ASPECTS } from "@/features/users/profileAspects";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The longest warning the server takes, in characters. */
const WARNING_MAX = 2000;

/**
 * What a reviewer does about an open case, any of: a warning in their own words, sent from
 * them (Message any user); a ban from the server (Ban users), with a deletion of recent
 * messages for a holder of Moderate any community; deleting the reported message (Moderate any
 * community); and resetting the reported aspects of a profile of this server's (Ban users). Each
 * is offered only to those who may take it. Done, the case is resolved for good.
 */
export function ResolveDialog({
  report: c,
  subject,
  subjectName,
  aspects,
  isOpen,
  onOpenChange,
  onResolved,
}: {
  report: ReportCase;
  subject: User | undefined;
  subjectName: string;
  /** The aspects the reports named, which a reset starts from. */
  aspects: readonly ProfileAspect[];
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  onResolved: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const mayWarn = useDeploymentCan("messageAnyUser");
  const mayBan = useDeploymentCan("banUsers");
  const moderator = useDeploymentCan("moderateCommunities");
  const foreign = subject?.homeDomain != null;
  const [warn, setWarn] = useState(false);
  const [warning, setWarning] = useState("");
  const [ban, setBan] = useState(false);
  const [banChoice, setBanChoice] = useState<BanChoice>(NEW_BAN);
  const [withOwner, setWithOwner] = useState(false);
  const [remove, setRemove] = useState(false);
  const [reset, setReset] = useState(false);
  const [resetting, setResetting] = useState<readonly ProfileAspect[]>(aspects);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const chosen =
    (warn && warning.trim() !== "") || ban || remove || (reset && resetting.length > 0);

  async function resolve() {
    if (!chosen || pending) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      await sync.admin.resolveReportCase(c.id, {
        ...(warn ? { warn: warning.trim() } : {}),
        ...(ban
          ? {
              ban: {
                ...banRequest(banChoice, moderator),
                withOwner: subject?.bot === true && withOwner,
              },
            }
          : {}),
        deleteMessage: remove,
        reset: reset ? [...resetting] : [],
      });
      toast(m.reports.resolvedToast);
      onOpenChange(false);
      onResolved();
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setPending(false);
    }
  }

  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={wideModalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{format(m.reports.resolveHeading, { name: subjectName })}</DialogHeading>
          <p className="text-sm text-ink-muted">{m.reports.resolveHint}</p>
          {mayWarn && (
            <div className="flex flex-col gap-2">
              <ChoiceCheckbox
                isSelected={warn}
                onChange={setWarn}
                label={m.reports.warn}
                hint={m.reports.warnHint}
              />
              {warn && (
                <TextField
                  value={warning}
                  onChange={setWarning}
                  maxLength={WARNING_MAX}
                  isRequired
                  className={fieldClass + " ms-6"}
                >
                  <Label className={labelClass}>{m.reports.warnTextLabel}</Label>
                  <TextArea
                    rows={3}
                    className={inputClass + " max-h-48 resize-none field-sizing-content"}
                  />
                </TextField>
              )}
            </div>
          )}
          {c.kind === "message" && moderator && (
            <ChoiceCheckbox
              isSelected={remove}
              onChange={setRemove}
              label={m.reports.deleteMessage}
              hint={m.reports.deleteMessageHint}
            />
          )}
          {c.kind === "profile" && mayBan && (
            <div className="flex flex-col gap-2">
              <ChoiceCheckbox
                isSelected={reset}
                onChange={setReset}
                isDisabled={foreign}
                label={m.reports.reset}
                hint={foreign ? m.reports.resetForeign : m.reports.resetHint}
              />
              {reset && (
                <div className="ms-6 grid grid-cols-2 gap-2 sm:grid-cols-3">
                  {PROFILE_ASPECTS.map((aspect) => (
                    <ChoiceCheckbox
                      key={aspect}
                      isSelected={resetting.includes(aspect)}
                      onChange={(selected) => {
                        setResetting((now) =>
                          selected ? [...now, aspect] : now.filter((a) => a !== aspect),
                        );
                      }}
                      label={m.reports.aspects[aspect]}
                    />
                  ))}
                </div>
              )}
            </div>
          )}
          {mayBan && (
            <div className="flex flex-col gap-2">
              <ChoiceCheckbox
                isSelected={ban}
                onChange={setBan}
                label={m.reports.ban}
                hint={m.reports.banHint}
              />
              {ban && (
                <div className="ms-6 flex flex-col gap-3">
                  <BanFields
                    value={banChoice}
                    onChange={setBanChoice}
                    mayDelete={moderator}
                    reasonHint={m.deployments.banReasonHint}
                    deleteLabel={m.deployments.banDeleteLabel}
                  />
                  {subject?.bot === true && (
                    <ChoiceCheckbox
                      isSelected={withOwner}
                      onChange={setWithOwner}
                      label={m.deployments.banWithOwner}
                      hint={m.deployments.banWithOwnerHint}
                    />
                  )}
                </div>
              )}
            </div>
          )}
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <div className="flex justify-end">
            <Button
              isDisabled={!chosen || pending}
              onPress={() => {
                void resolve();
              }}
              className={primaryButtonClass}
            >
              {pending ? m.reports.resolving : m.reports.resolve}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
