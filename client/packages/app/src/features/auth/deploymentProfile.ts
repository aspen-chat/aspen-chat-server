import type { AspenClient, DeploymentProfile } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useAspenClient } from "@/api/context";

/**
 * The `id` of the element holding the profile in the head of the page a deployment serves
 * (`api::web_client`; both match `spec/web_client_page.json`).
 */
export const PROFILE_ELEMENT_ID = "aspen-deployment-profile";

/**
 * The profile the page was served with, when it names the deployment at `baseUrl`, whose own
 * page it then is. Only a deployment's own server writes it, so it is taken as `GET /deployment`
 * answers; something other than a profile is left alone.
 */
export function embeddedProfile(doc: Document, baseUrl: string): DeploymentProfile | null {
  const text = doc.getElementById(PROFILE_ELEMENT_ID)?.textContent;
  if (text == null) {
    return null;
  }
  let profile: unknown;
  try {
    profile = JSON.parse(text);
  } catch {
    return null;
  }
  if (
    typeof profile !== "object" ||
    profile === null ||
    !("webClientUrl" in profile) ||
    typeof profile.webClientUrl !== "string" ||
    !URL.canParse(profile.webClientUrl) ||
    !URL.canParse(baseUrl) ||
    new URL(profile.webClientUrl).origin !== new URL(baseUrl).origin
  ) {
    return null;
  }
  return profile as DeploymentProfile;
}

/**
 * The last profile each client read, by its server: what a component shows while it reads the
 * profile afresh. The page's own profile stands in for its deployment's until the first read.
 */
const lastRead = new WeakMap<AspenClient, DeploymentProfile>();

function known(client: AspenClient): DeploymentProfile | undefined {
  const read = lastRead.get(client);
  if (read !== undefined) {
    return read;
  }
  const embedded =
    typeof document === "undefined" ? null : embeddedProfile(document, client.baseUrl);
  if (embedded !== null) {
    lastRead.set(client, embedded);
  }
  return embedded ?? undefined;
}

/**
 * The deployment's profile, read afresh each time a component using it mounts. Until the read
 * answers it is the profile last read from that server, or the one the page was served with,
 * and `undefined` when there is neither; a failed read keeps what is known, or is empty.
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
    client.deploymentProfile().then(
      (profile) => {
        lastRead.set(client, profile);
        settle(profile);
      },
      () => {
        // A deployment's web client is at its API's origin.
        settle(known(client) ?? { displayName: null, icon: null, webClientUrl: client.baseUrl });
      },
    );
    return () => {
      current = false;
    };
  }, [client]);
  return read?.server === client.baseUrl ? read.profile : known(client);
}
