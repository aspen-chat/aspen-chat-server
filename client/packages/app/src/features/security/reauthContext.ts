import { createContext, useContext } from "react";

/**
 * Runs a change to security settings, asking the user to confirm it's them first when the server
 * says the session's last verification is too old, then trying once more. Resolves to
 * `undefined` when they decline.
 */
export type WithReauth = <T>(action: () => Promise<T>) => Promise<T | undefined>;

export const ReauthContext = createContext<WithReauth | null>(null);

/**
 * `useReauth` where a provider may be absent (a screen shown signed in and signed out alike):
 * runs the action as it is when there is none.
 */
export function useOptionalReauth(): WithReauth {
  return useContext(ReauthContext) ?? withoutReauth;
}

/** Runs the action as it is: one function, so hooks that depend on it stay still. */
const withoutReauth: WithReauth = (action) => action();

export function useReauth(): WithReauth {
  const withReauth = useContext(ReauthContext);
  if (withReauth === null) {
    throw new Error("useReauth must be used inside <ReauthProvider>");
  }
  return withReauth;
}
