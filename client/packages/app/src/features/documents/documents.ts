import type { DocumentKind, PublishedDocument } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useAspenClient } from "@/api/context";
import type { Messages } from "@/i18n/messages";

/** What each document is called, and what shows where a deployment has not published it. */
export function documentNames(m: Messages, kind: DocumentKind): { title: string; none: string } {
  return kind === "privacyPolicy"
    ? { title: m.documents.privacyPolicy, none: m.documents.noPrivacyPolicy }
    : { title: m.documents.termsOfUse, none: m.documents.noTermsOfUse };
}

/**
 * The deployment's document, read afresh each time a component using it mounts; `undefined`
 * until it is read, and `null` when the read fails.
 */
export function useDeploymentDocument(kind: DocumentKind): PublishedDocument | null | undefined {
  const client = useAspenClient();
  const [read, setRead] = useState<
    { server: string; kind: DocumentKind; document: PublishedDocument | null } | undefined
  >();
  useEffect(() => {
    let current = true;
    const settle = (document: PublishedDocument | null) => {
      if (current) {
        setRead({ server: client.baseUrl, kind, document });
      }
    };
    client.deploymentDocument(kind).then(settle, () => {
      settle(null);
    });
    return () => {
      current = false;
    };
  }, [client, kind]);
  return read?.server === client.baseUrl && read.kind === kind ? read.document : undefined;
}
