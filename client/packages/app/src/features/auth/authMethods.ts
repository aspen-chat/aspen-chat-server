import type { AspenClient, AuthMethods } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useAspenClient } from "@/api/context";

const methodsByServer = new Map<string, Promise<AuthMethods>>();

/**
 * How the server lets people sign in and register (`GET /auth/methods`), asked once per server.
 * A failed ask is asked again next time.
 */
export function authMethods(client: AspenClient): Promise<AuthMethods> {
  let pending = methodsByServer.get(client.baseUrl);
  if (pending === undefined) {
    pending = client.authMethods();
    pending.catch(() => methodsByServer.delete(client.baseUrl));
    methodsByServer.set(client.baseUrl, pending);
  }
  return pending;
}

/** The current server's sign-in and registration methods; `null` until they are known. */
export function useAuthMethods(): AuthMethods | null {
  const client = useAspenClient();
  const [methods, setMethods] = useState<{ server: string; methods: AuthMethods } | null>(null);
  useEffect(() => {
    let current = true;
    authMethods(client).then(
      (found) => {
        if (current) {
          setMethods({ server: client.baseUrl, methods: found });
        }
      },
      () => undefined,
    );
    return () => {
      current = false;
    };
  }, [client]);
  return methods?.server === client.baseUrl ? methods.methods : null;
}
