import type { DocumentKind } from "@aspen/protocol";
import { Link } from "@tanstack/react-router";
import { linkButtonClass } from "@/features/auth/styles";
import { documentNames, useDeploymentDocument } from "@/features/documents/documents";
import { DocumentText } from "@/features/documents/DocumentText";
import { useDocumentTitle } from "@/features/layout/documentTitle";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const DATE: Intl.DateTimeFormatOptions = { dateStyle: "long" };

/**
 * A deployment's Privacy Policy or Terms of Use (`/privacy`, `/terms`), signed in or out: the
 * home deployment's signed in, and the one the sign-in screen names signed out, read afresh each
 * time the page opens. A deployment that has published none says so in words of the app's own;
 * Aspen gives no default. Signed out, the page leads back to the sign-in screen.
 */
export function DocumentScreen({
  kind,
  signedOut = false,
}: {
  kind: DocumentKind;
  signedOut?: boolean;
}) {
  const m = useMessages();
  const names = documentNames(m, kind);
  useDocumentTitle(names.title);
  const document = useDeploymentDocument(kind);
  const date = useDateFormat(DATE);
  return (
    <main className="h-full min-h-0 min-w-0 flex-1 overflow-y-auto bg-surface">
      <article className="mx-auto flex max-w-3xl flex-col gap-4 px-4 py-6 md:px-6">
        {signedOut && (
          <Link to="/" className={linkButtonClass + " self-start text-sm"}>
            {m.about.backToSignIn}
          </Link>
        )}
        <h1 className="text-xl font-semibold">{names.title}</h1>
        {document === undefined ? (
          <div aria-busy="true" className="flex flex-col gap-2">
            <LoadingLabel />
            {Array.from({ length: 6 }, (_, i) => (
              <Skeleton key={i} className="h-4 w-full" />
            ))}
          </div>
        ) : document === null ? (
          <p role="alert" className="text-sm text-danger">
            {m.documents.loadFailed}
          </p>
        ) : document.body == null ? (
          <p>{names.none}</p>
        ) : (
          <>
            {document.updatedAt != null && (
              <p className="text-sm text-ink-muted">
                {format(m.documents.updated, { date: date.format(new Date(document.updatedAt)) })}
              </p>
            )}
            <DocumentText body={document.body} />
          </>
        )}
      </article>
    </main>
  );
}
