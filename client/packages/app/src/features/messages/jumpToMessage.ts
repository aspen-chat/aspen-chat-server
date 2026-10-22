import { createContext, useContext } from "react";

/**
 * Brings a message of `channelId` into the view of the message list showing it, and flashes it,
 * as a reply's line does for the message it answers (`MessageList`). `null` outside a list.
 */
export type JumpToMessage = (channelId: string, messageId: string) => void;

export const JumpToMessageContext = createContext<JumpToMessage | null>(null);

export function useJumpToMessage(): JumpToMessage | null {
  return useContext(JumpToMessageContext);
}
