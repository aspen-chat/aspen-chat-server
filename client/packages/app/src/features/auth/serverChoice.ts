import { createContext, useContext } from "react";

/** The chosen server and a way back to the server picker, for the sign-in screen. */
export interface ServerChoice {
  serverUrl: string;
  changeServer: () => void;
}

export const ServerChoiceContext = createContext<ServerChoice | null>(null);

export function useServerChoice(): ServerChoice {
  const choice = useContext(ServerChoiceContext);
  if (choice === null) {
    throw new Error("useServerChoice must be used inside <App>");
  }
  return choice;
}
