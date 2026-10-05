import { useEffect, useState } from "react";
import { useAspenClient } from "@/api/context";
import { fetchPolicy, type EmailPolicy } from "./api";

/** What a deployment that could not be asked is taken to do with email: nothing. */
const NONE: EmailPolicy = {
  available: false,
  required: false,
  verificationRequired: false,
  newsletter: false,
};

/**
 * What the current server does with email, read each time a screen that offers it opens, since
 * its administrators may change it at any time; `null` until it is known. A failed read is taken
 * as no email, so nothing is offered that the server would refuse.
 */
export function useEmailPolicy(): EmailPolicy | null {
  const client = useAspenClient();
  const [policy, setPolicy] = useState<{ server: string; policy: EmailPolicy } | null>(null);
  useEffect(() => {
    let live = true;
    fetchPolicy(client).then(
      (read) => {
        if (live) {
          setPolicy({ server: client.baseUrl, policy: read });
        }
      },
      () => {
        if (live) {
          setPolicy({ server: client.baseUrl, policy: NONE });
        }
      },
    );
    return () => {
      live = false;
    };
  }, [client]);
  return policy?.server === client.baseUrl ? policy.policy : null;
}
