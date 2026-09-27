import { createContext, useContext } from "react";

/**
 * Runs a change to security settings, asking the user to confirm it's them first when the server
 * says the session's last verification is too old, then trying once more. Resolves to
 * `undefined` when they decline.
 */
export type WithReauth = <T>(action: () => Promise<T>) => Promise<T | undefined>;

export const ReauthContext = createContext<WithReauth | null>(null);

export function useReauth(): WithReauth {
  const withReauth = useContext(ReauthContext);
  if (withReauth === null) {
    throw new Error("useReauth must be used inside <ReauthProvider>");
  }
  return withReauth;
}
