import { createContext, useContext } from "react";
import { en, type Messages } from "./messages";

export const MessagesContext = createContext<Messages>(en);

export function useMessages(): Messages {
  return useContext(MessagesContext);
}
