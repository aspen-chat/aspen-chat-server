import { ApiProblemError } from "@aspen/protocol";
import { ArrowBendLeftUpIcon } from "@phosphor-icons/react";
import { Button } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { toast } from "@/features/layout/toast";
import { Tooltip } from "@/features/layout/Tooltip";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { useEchoTarget } from "@/features/threads/echoTarget";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * Shows a thread reply that was posted without an echo in the thread's parent channel, as the
 * message box's checkbox would have; offered to the reply's author until it has one. A toast
 * says where it went, since on a phone the parent channel is not on screen, or why it could
 * not.
 */
export function EchoReplyButton({
  messageId,
  parentId,
  className,
  onPressed,
}: {
  messageId: string;
  parentId: string;
  className: string;
  /** Called as it is pressed, for a popover offering it to close. */
  onPressed?: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const target = useEchoTarget(parentId);
  const label = format(m.threads.echoToParent, { channel: target });
  return (
    <Tooltip text={label}>
      <Button
        aria-label={label}
        onPress={() => {
          sync.echoReply(messageId).then(
            () => {
              toast(format(m.threads.echoed, { channel: target }));
            },
            (problem: unknown) => {
              toast(problem instanceof ApiProblemError ? problem.message : m.threads.echoFailed);
            },
          );
          onPressed?.();
        }}
        className={className}
      >
        <ArrowBendLeftUpIcon size={ACTION_ICON} aria-hidden="true" className="rtl:-scale-x-100" />
      </Button>
    </Tooltip>
  );
}
