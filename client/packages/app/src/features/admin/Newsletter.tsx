import type { NewsletterPost } from "@aspen/protocol";
import { PaperPlaneTiltIcon, PencilSimpleIcon, PlusIcon, TrashIcon } from "@phosphor-icons/react";
import { useCallback, useState } from "react";
import {
  Button,
  Dialog,
  FieldError,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  Text,
  TextArea,
  TextField,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { useFigures } from "@/features/admin/format";
import { useAdminRead } from "@/features/admin/useAdminRead";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  accentButtonClass,
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
  planeSurfaceClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { LoadingLabel } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

/** The longest subject and body the server takes, in characters. */
const SUBJECT_MAX = 200;
const BODY_MAX = 100_000;

/**
 * The deployment's newsletter, for holders of Send newsletters: the posts, newest first, drafts
 * and the archive of those sent; a draft written in Markdown, previewed as the mail shows it,
 * sent as a test to the sender's own verified address, and sent to every subscriber behind a
 * confirmation, after which it is fixed.
 */
export function NewsletterSection() {
  const m = useMessages();
  const sync = useSync();
  const load = useCallback(() => sync.admin.newsletterPosts(), [sync]);
  const posts = useAdminRead(load);
  // The post open in the editor: a new draft (`null`), or one of the list.
  const [editing, setEditing] = useState<NewsletterPost | null | undefined>(undefined);
  return (
    <Section id="newsletter" title={m.email.newsletterTitle} hint={m.email.newsletterAdminHint}>
      {editing !== undefined ? (
        <PostEditor
          post={editing}
          onSaved={(post) => {
            setEditing(post);
            posts.reload();
          }}
          onClose={() => {
            setEditing(undefined);
            posts.reload();
          }}
        />
      ) : (
        <>
          <Button
            onPress={() => {
              setEditing(null);
            }}
            className={accentButtonClass + " flex items-center gap-1.5 self-start"}
          >
            <PlusIcon size={16} aria-hidden="true" />
            {m.email.newPost}
          </Button>
          {posts.error !== null && <ReadFailed error={posts.error} onRetry={posts.reload} />}
          {posts.data === undefined && posts.error === null && <LoadingLabel />}
          {posts.data !== undefined && (
            <PostList posts={posts.data} onOpen={setEditing} onChanged={posts.reload} />
          )}
        </>
      )}
    </Section>
  );
}

function PostList({
  posts,
  onOpen,
  onChanged,
}: {
  posts: NewsletterPost[];
  onOpen: (post: NewsletterPost) => void;
  onChanged: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const when = useDateFormat({ dateStyle: "medium", timeStyle: "short" });
  const figures = useFigures();
  const [deleting, setDeleting] = useState<NewsletterPost | null>(null);
  const [error, setError] = useState<string | null>(null);
  if (posts.length === 0) {
    return <p className={hintClass}>{m.email.noPosts}</p>;
  }
  return (
    <>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <ul className="flex flex-col gap-2">
        {posts.map((post) => (
          <li
            key={post.id}
            className={planeSurfaceClass + " flex flex-wrap items-center justify-between gap-2"}
          >
            <div className="min-w-0">
              <p className="font-medium break-words">{post.subject}</p>
              <p className={hintClass}>
                {post.sentAt == null
                  ? format(m.email.draftEdited, { when: when.format(new Date(post.updatedAt)) })
                  : post.queuedAt == null
                    ? format(m.email.postSending, {
                        when: when.format(new Date(post.sentAt)),
                        count: figures.count(post.recipients),
                      })
                    : format(m.email.postSent, {
                        when: when.format(new Date(post.sentAt)),
                        count: figures.count(post.recipients),
                      })}
              </p>
            </div>
            <div className="flex gap-2">
              <Button
                onPress={() => {
                  onOpen(post);
                }}
                className={secondaryButtonClass + " flex items-center gap-1.5"}
              >
                <PencilSimpleIcon size={14} aria-hidden="true" />
                {post.sentAt == null ? m.email.editPost : m.email.viewPost}
              </Button>
              {post.sentAt == null && (
                <Button
                  onPress={() => {
                    setError(null);
                    setDeleting(post);
                  }}
                  className={secondaryButtonClass + " flex items-center gap-1.5 text-danger"}
                >
                  <TrashIcon size={14} aria-hidden="true" />
                  {m.email.deletePost}
                </Button>
              )}
            </div>
          </li>
        ))}
      </ul>
      <Confirm
        open={deleting !== null}
        heading={m.email.deletePostHeading}
        text={format(m.email.deletePostHint, { subject: deleting?.subject ?? "" })}
        action={m.email.deletePost}
        onClose={() => {
          setDeleting(null);
        }}
        onConfirm={async () => {
          if (deleting === null) {
            return;
          }
          try {
            await sync.admin.deleteNewsletterPost(deleting.id);
            setDeleting(null);
            onChanged();
          } catch (e) {
            setDeleting(null);
            setError(problemText(e));
          }
        }}
      />
    </>
  );
}

/**
 * A post being written or read: its subject and Markdown body, saved as a draft, previewed as the
 * mail shows it, and sent. A sent post shows, fixed.
 */
function PostEditor({
  post,
  onSaved,
  onClose,
}: {
  post: NewsletterPost | null;
  onSaved: (post: NewsletterPost) => void;
  onClose: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const sent = post?.sentAt != null;
  const [subject, setSubject] = useState(post?.subject ?? "");
  const [body, setBody] = useState(post?.body ?? "");
  const [pending, setPending] = useState<"save" | "test" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const unsaved = post?.subject !== subject || post.body !== body;

  async function save(): Promise<NewsletterPost | null> {
    setPending("save");
    setError(null);
    setStatus(null);
    try {
      const saved =
        post === null
          ? await sync.admin.createNewsletterPost({ subject, body })
          : await sync.admin.updateNewsletterPost(post.id, { subject, body });
      onSaved(saved);
      setStatus(m.email.draftSaved);
      return saved;
    } catch (e) {
      setError(problemText(e));
      return null;
    } finally {
      setPending(null);
    }
  }

  async function test() {
    const saved = unsaved ? await save() : post;
    if (saved === null) {
      return;
    }
    setPending("test");
    setError(null);
    try {
      await sync.admin.testNewsletterPost(saved.id);
      setStatus(m.email.testSent);
    } catch (e) {
      setError(problemText(e));
    } finally {
      setPending(null);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <Form
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
        className="flex flex-col gap-3"
      >
        <TextField
          value={subject}
          onChange={setSubject}
          isRequired
          isReadOnly={sent}
          maxLength={SUBJECT_MAX}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.email.postSubject}</Label>
          <Input className={inputClass} />
          <FieldError className="text-sm text-danger" />
        </TextField>
        <TextField
          value={body}
          onChange={setBody}
          isRequired
          isReadOnly={sent}
          maxLength={BODY_MAX}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.email.postBody}</Label>
          <TextArea rows={12} className={inputClass + " resize-y font-mono text-sm"} />
          <Text slot="description" className={hintClass}>
            {m.email.postBodyHint}
          </Text>
          <FieldError className="text-sm text-danger" />
        </TextField>
        {error !== null && (
          <p role="alert" className={alertClass}>
            {error}
          </p>
        )}
        {status !== null && error === null && (
          <p role="status" className={hintClass}>
            {status}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          {!sent && (
            <>
              <Button
                type="submit"
                isDisabled={pending !== null || !unsaved}
                className={primaryButtonClass}
              >
                {pending === "save" ? m.email.saving : m.email.saveDraft}
              </Button>
              <Button
                isDisabled={pending !== null}
                onPress={() => {
                  void test();
                }}
                className={secondaryButtonClass}
              >
                {pending === "test" ? m.email.sendingTest : m.email.sendTest}
              </Button>
              <Button
                isDisabled={pending !== null || post === null || unsaved}
                onPress={() => {
                  setError(null);
                  setConfirming(true);
                }}
                className={accentButtonClass + " flex items-center gap-1.5"}
              >
                <PaperPlaneTiltIcon size={16} aria-hidden="true" />
                {m.email.sendToAll}
              </Button>
            </>
          )}
          <Button onPress={onClose} className={secondaryButtonClass}>
            {m.email.backToPosts}
          </Button>
        </div>
        {!sent && post !== null && unsaved && <p className={hintClass}>{m.email.saveFirst}</p>}
      </Form>
      {post !== null && !unsaved && (
        <section aria-labelledby="newsletter-preview" className="flex flex-col gap-2">
          <h3 id="newsletter-preview" className="text-sm font-semibold text-ink-muted">
            {m.email.preview}
          </h3>
          {/* The server's HTML, in a frame that runs no script and reaches nothing of this
              page, so even a mistake in it cannot act here. */}
          <iframe
            title={m.email.preview}
            sandbox=""
            srcDoc={`<!doctype html><meta charset="utf-8"><body style="font-family:system-ui,sans-serif;margin:16px;line-height:1.5">${post.html}</body>`}
            className="h-80 w-full rounded-md border border-line bg-white"
          />
        </section>
      )}
      <Confirm
        open={confirming}
        heading={m.email.sendHeading}
        text={format(m.email.sendHint, { subject })}
        action={m.email.sendToAll}
        onClose={() => {
          setConfirming(false);
        }}
        onConfirm={async () => {
          if (post === null) {
            return;
          }
          try {
            onSaved(await sync.admin.sendNewsletterPost(post.id));
            setStatus(m.email.postQueued);
          } catch (e) {
            setError(problemText(e));
          } finally {
            setConfirming(false);
          }
        }}
      />
    </div>
  );
}

/** Asks before something that cannot be undone. */
function Confirm({
  open,
  heading,
  text,
  action,
  onClose,
  onConfirm,
}: {
  open: boolean;
  heading: string;
  text: string;
  action: string;
  onClose: () => void;
  onConfirm: () => Promise<void>;
}) {
  const [pending, setPending] = useState(false);
  return (
    <ModalOverlay
      isOpen={open}
      onOpenChange={(isOpen) => {
        if (!isOpen) {
          onClose();
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role="alertdialog" className={dialogClass}>
          <DialogHeading>{heading}</DialogHeading>
          <p className="text-sm text-ink-muted">{text}</p>
          <Button
            isDisabled={pending}
            onPress={() => {
              setPending(true);
              void onConfirm().finally(() => {
                setPending(false);
              });
            }}
            className={dangerButtonClass + " self-end"}
          >
            {action}
          </Button>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
