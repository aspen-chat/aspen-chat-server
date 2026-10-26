import type { DocumentKind } from "@aspen/protocol";
import { Link } from "@tanstack/react-router";
import { useCallback, useState } from "react";
import { Button, Form, Label, Text, TextArea, TextField } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { useAdminRead } from "@/features/admin/useAdminRead";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  linkButtonClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { documentNames } from "@/features/documents/documents";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";

/** The longest document, in characters, as the server's `deployment_document::MAX_CHARS`. */
const MAX_CHARS = 65_536;

/**
 * The deployment's Privacy Policy and Terms of Use, for holders of Manage deployment settings:
 * each written in Markdown, published for anyone to read at `/privacy` and `/terms`, which the
 * sign-in screen links to, or withdrawn. Aspen gives no default for either.
 */
export function DeploymentDocumentsSection() {
  const m = useMessages();
  return (
    <Section id="deployment-documents" title={m.admin.documentsTitle} hint={m.admin.documentsHint}>
      <DocumentForm kind="privacyPolicy" page="/privacy" />
      <DocumentForm kind="termsOfUse" page="/terms" />
    </Section>
  );
}

/** One document's editor, starting from what the deployment has published. */
function DocumentForm({ kind, page }: { kind: DocumentKind; page: "/privacy" | "/terms" }) {
  const m = useMessages();
  const client = useAspenClient();
  const load = useCallback(() => client.deploymentDocument(kind), [client, kind]);
  const read = useAdminRead(load);
  const names = documentNames(m, kind);
  if (read.error !== null) {
    return <ReadFailed error={read.error} onRetry={read.reload} />;
  }
  if (read.data === undefined) {
    return null;
  }
  return (
    <DocumentEditor
      kind={kind}
      title={names.title}
      page={page}
      saved={read.data.body ?? null}
      reload={read.reload}
    />
  );
}

function DocumentEditor({
  kind,
  title,
  page,
  saved,
  reload,
}: {
  kind: DocumentKind;
  title: string;
  page: "/privacy" | "/terms";
  /** The document as published, `null` for none. The field keeps what was typed as it changes. */
  saved: string | null;
  reload: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [body, setBody] = useState(saved ?? "");
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run(work: () => Promise<unknown>, done: string) {
    setBusy(true);
    setStatus(null);
    setError(null);
    try {
      await work();
      setStatus(done);
      reload();
    } catch (e) {
      setError(problemText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        void run(() => sync.admin.publishDeploymentDocument(kind, body), m.admin.documentPublished);
      }}
      className="flex flex-col gap-3"
    >
      <TextField
        value={body}
        onChange={(value) => {
          setBody(value);
          setStatus(null);
        }}
        maxLength={MAX_CHARS}
        className={fieldClass}
      >
        <Label className={labelClass}>{title}</Label>
        <TextArea rows={10} className={inputClass + " resize-y font-mono text-sm"} />
        <Text slot="description" className={hintClass}>
          {saved === null ? m.admin.documentUnpublished : m.admin.documentMarkdownHint}
        </Text>
      </TextField>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <Button
          type="submit"
          isDisabled={busy || body.trim() === "" || body === saved}
          className={primaryButtonClass}
        >
          {m.admin.publishDocument}
        </Button>
        {saved !== null && (
          <Button
            isDisabled={busy}
            onPress={() => {
              void run(
                () => sync.admin.withdrawDeploymentDocument(kind),
                m.admin.documentWithdrawn,
              );
            }}
            className={secondaryButtonClass}
          >
            {m.admin.withdrawDocument}
          </Button>
        )}
        <Link to={page} className={linkButtonClass + " text-sm"}>
          {m.admin.viewDocument}
        </Link>
        {status !== null && (
          <span role="status" className="text-sm text-ink-muted">
            {status}
          </span>
        )}
      </div>
    </Form>
  );
}
