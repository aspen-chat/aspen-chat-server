import { createContext, useContext } from "react";

/**
 * Whether the member list is shown, owned by the community layout and toggled from a channel's
 * header: the pane beside the channel on a large screen, the drawer over it on a smaller one.
 */
export interface MembersPanel {
  open: boolean;
  toggle: () => void;
}

export const MembersPanelContext = createContext<MembersPanel>({
  open: true,
  toggle: () => undefined,
});

export function useMembersPanel(): MembersPanel {
  return useContext(MembersPanelContext);
}
