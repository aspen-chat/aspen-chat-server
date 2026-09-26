import { createContext, useContext } from "react";

/** Whether the members panel is shown, owned by the community layout and toggled from a channel's header. */
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
