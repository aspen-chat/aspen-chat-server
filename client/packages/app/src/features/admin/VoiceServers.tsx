import type { VoiceServer } from "@aspen/protocol";
import { useCallback, useState } from "react";
import { Button, Form, Input, Label, NumberField, Text, TextField } from "react-aria-components";
import { problemText } from "@/api/problemText";
import { useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { useAdminRead } from "@/features/admin/useAdminRead";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { copyText } from "@/features/layout/clipboard";
import { ConfirmModal } from "@/features/layout/ConfirmModal";
import { Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The most participants a voice server may be given, as the server's `i32`. */
const MAX_CAPACITY = 2_147_483_647;

/** What the form for a voice server holds. */
interface Draft {
  name: string;
  url: string;
  capacity: number;
}

/**
 * The registered voice servers, for holders of Manage voice servers: adding one, changing its
 * name, address, capacity, and whether new calls may start on it, and removing it, which ends
 * any call on it. Each shows what the voice server must be given to report as itself: its id,
 * for its `voice_server.toml`, and its NATS user's entry. The health of each is under Fleet.
 */
export function VoiceServersSection() {
  const m = useMessages();
  const v = m.admin.voiceServerAdmin;
  const sync = useSync();
  const load = useCallback(() => sync.admin.voiceServers(), [sync]);
  const read = useAdminRead(load);
  const list = read.data;
  // The server just added shows how to set it up at once; the others when asked.
  const [added, setAdded] = useState<string | null>(null);
  return (
    <Section id="admin-voice-servers" title={m.admin.voiceServers} hint={v.hint}>
      {read.error !== null && <ReadFailed error={read.error} onRetry={read.reload} />}
      {list === undefined ? (
        read.error === null && (
          <div aria-busy="true" className="flex flex-col gap-2">
            <Skeleton className="h-14 w-full" />
            <Skeleton className="h-14 w-full" />
          </div>
        )
      ) : list.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.admin.noVoiceServers}</p>
      ) : (
        <ul className="flex flex-col divide-y divide-line rounded-md border border-line">
          {list.map((server) => (
            <li key={server.id}>
              <ServerRow server={server} setupShown={added === server.id} onChanged={read.reload} />
            </li>
          ))}
        </ul>
      )}
      <AddServer
        onAdded={(server) => {
          setAdded(server.id);
          read.reload();
        }}
      />
    </Section>
  );
}

function ServerRow({
  server,
  setupShown,
  onChanged,
}: {
  server: VoiceServer;
  setupShown: boolean;
  onChanged: () => void;
}) {
  const m = useMessages();
  const v = m.admin.voiceServerAdmin;
  const sync = useSync();
  const [editing, setEditing] = useState(false);
  const [setup, setSetup] = useState(setupShown);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="flex flex-col gap-2 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1">
          <span className="block font-medium break-words">{server.name}</span>
          <span className="block text-xs break-all text-ink-muted">{server.url}</span>
          <span className="block text-xs text-ink-muted">
            {format(v.capacity, { capacity: server.capacity })}
            {server.enabled ? "" : ` · ${v.disabled}`}
          </span>
        </span>
        <Button
          onPress={() => {
            setSetup((shown) => !shown);
          }}
          aria-expanded={setup}
          className={secondaryButtonClass}
        >
          {v.setup}
        </Button>
        <Button
          onPress={() => {
            setEditing((open) => !open);
          }}
          aria-expanded={editing}
          className={secondaryButtonClass}
        >
          {v.edit}
        </Button>
        <Button
          onPress={() => {
            setRemoving(true);
          }}
          className={secondaryButtonClass + " text-danger"}
        >
          {v.remove}
        </Button>
      </div>
      <ChoiceCheckbox
        isSelected={server.enabled}
        onChange={(enabled) => {
          setError(null);
          sync.admin.updateVoiceServer(server.id, { enabled }).then(onChanged, (e: unknown) => {
            setError(problemText(e));
          });
        }}
        label={v.enabled}
        hint={v.enabledHint}
      />
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      {setup && <Setup server={server} />}
      {editing && (
        <ServerForm
          initial={{ name: server.name, url: server.url, capacity: server.capacity }}
          submit={v.save}
          save={async (draft) => {
            await sync.admin.updateVoiceServer(server.id, draft);
            setEditing(false);
            onChanged();
          }}
        />
      )}
      <ConfirmModal
        isOpen={removing}
        onOpenChange={setRemoving}
        heading={format(v.removeHeading, { name: server.name })}
        explanation={v.removeExplanation}
        action={v.remove}
        run={async () => {
          await sync.admin.deleteVoiceServer(server.id);
          onChanged();
        }}
      />
    </div>
  );
}

/** What the voice server must be given to report as itself, each with a way to copy it. */
function Setup({ server }: { server: VoiceServer }) {
  const m = useMessages();
  const v = m.admin.voiceServerAdmin;
  return (
    <div className="flex flex-col gap-3 rounded-md bg-surface-sunken p-3">
      <Copyable label={v.idLabel} hint={v.idHint} text={`id = "${server.id}"`} />
      <Copyable label={v.natsLabel} hint={v.natsHint} text={server.natsUser} />
    </div>
  );
}

function Copyable({ label, hint, text }: { label: string; hint: string; text: string }) {
  const m = useMessages();
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex flex-col gap-1">
      <span className={labelClass}>{label}</span>
      <span className={hintClass}>{hint}</span>
      <pre className="overflow-x-auto rounded border border-line bg-surface p-2 font-mono text-xs">
        {text}
      </pre>
      <Button
        onPress={(event) => {
          void copyText(text, event.target).then((done) => {
            setCopied(done);
          });
        }}
        className={secondaryButtonClass + " self-start"}
      >
        {copied ? m.copied : m.admin.voiceServerAdmin.copy}
      </Button>
    </div>
  );
}

function AddServer({ onAdded }: { onAdded: (server: VoiceServer) => void }) {
  const m = useMessages();
  const v = m.admin.voiceServerAdmin;
  const sync = useSync();
  const [open, setOpen] = useState(false);
  if (!open) {
    return (
      <Button
        onPress={() => {
          setOpen(true);
        }}
        className={secondaryButtonClass + " self-start"}
      >
        {v.add}
      </Button>
    );
  }
  return (
    <ServerForm
      initial={{ name: "", url: "", capacity: 100 }}
      submit={v.add}
      save={async (draft) => {
        const server = await sync.admin.createVoiceServer(draft);
        setOpen(false);
        onAdded(server);
      }}
    />
  );
}

/** A voice server's name, address, and capacity, saved by `save`, whose failure it shows. */
function ServerForm({
  initial,
  submit,
  save,
}: {
  initial: Draft;
  submit: string;
  save: (draft: Draft) => Promise<void>;
}) {
  const v = useMessages().admin.voiceServerAdmin;
  const [draft, setDraft] = useState(initial);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        setPending(true);
        setError(null);
        save({ ...draft, name: draft.name.trim(), url: draft.url.trim() }).then(
          () => {
            setPending(false);
          },
          (e: unknown) => {
            setPending(false);
            setError(problemText(e));
          },
        );
      }}
      className="flex flex-col gap-3 rounded-md border border-line p-3"
    >
      <TextField
        value={draft.name}
        onChange={(name) => {
          setDraft((d) => ({ ...d, name }));
        }}
        isRequired
        className={fieldClass}
      >
        <Label className={labelClass}>{v.name}</Label>
        <Input className={inputClass} />
        <Text slot="description" className={hintClass}>
          {v.nameHint}
        </Text>
      </TextField>
      <TextField
        value={draft.url}
        onChange={(url) => {
          setDraft((d) => ({ ...d, url }));
        }}
        isRequired
        className={fieldClass}
      >
        <Label className={labelClass}>{v.url}</Label>
        <Input placeholder="https://voice-1.example.org" className={inputClass} />
        <Text slot="description" className={hintClass}>
          {v.urlHint}
        </Text>
      </TextField>
      <NumberField
        value={draft.capacity}
        onChange={(capacity) => {
          setDraft((d) => ({ ...d, capacity: Number.isNaN(capacity) ? 0 : capacity }));
        }}
        minValue={1}
        maxValue={MAX_CAPACITY}
        isRequired
        className={fieldClass}
      >
        <Label className={labelClass}>{v.capacityLabel}</Label>
        <Input className={inputClass} />
        <Text slot="description" className={hintClass}>
          {v.capacityHint}
        </Text>
      </NumberField>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass + " self-start"}>
        {submit}
      </Button>
    </Form>
  );
}
