import type { DeploymentProfile } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useAspenClient } from "@/api/context";

/**
 * The deployment's profile, read afresh each time a component using it mounts; `undefined`
 * until it is read, and empty when the read fails.
 */
export function useDeploymentProfile(): DeploymentProfile | undefined {
  const client = useAspenClient();
  const [read, setRead] = useState<{ server: string; profile: DeploymentProfile } | undefined>();
  useEffect(() => {
    let current = true;
    const settle = (profile: DeploymentProfile) => {
      if (current) {
        setRead({ server: client.baseUrl, profile });
      }
    };
    client.deploymentProfile().then(settle, () => {
      // A deployment's web client is at its API's origin.
      settle({ displayName: null, icon: null, webClientUrl: client.baseUrl });
    });
    return () => {
      current = false;
    };
  }, [client]);
  return read?.server === client.baseUrl ? read.profile : undefined;
}
