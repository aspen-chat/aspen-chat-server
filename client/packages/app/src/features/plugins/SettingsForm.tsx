import type { PluginInfo, SettingField } from "@aspen/protocol";
import { pluginKey } from "@aspen/protocol";
import { CaretDownIcon } from "@phosphor-icons/react";
import { useState, type ReactNode } from "react";
import {
  Button,
  Form,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  NumberField,
  Popover,
  Select,
  SelectValue,
  Text,
  TextArea,
  TextField,
} from "react-aria-components";
import { useChannels, useRoles } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { optionClass, selectButtonClass, selectPopoverClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";

/** A value as a form holds it, before it becomes what the server takes. */
type Draft = string | boolean | number | readonly string[] | null;

const NONE = "";

/** What the form starts from for `field`: the value set, its default, or nothing. */
function initial(field: SettingField, values: Record<string, unknown>): Draft {
  const value = values[field.name] ?? field.default ?? null;
  switch (field.type) {
    case "boolean":
      return value === true;
    case "integer":
      return typeof value === "number" ? value : null;
    case "textList":
    case "roleList":
    case "channelList":
      return Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : [];
    default:
      return typeof value === "string" ? value : null;
  }
}

/** Whether two drafts hold the same. */
function same(a: Draft, b: Draft): boolean {
  if (isList(a) && isList(b)) {
    return a.length === b.length && a.every((v, i) => v === b[i]);
  }
  return a === b;
}

/** Whether a draft is a list. */
function isList(draft: Draft): draft is readonly string[] {
  return Array.isArray(draft);
}

/** What the server takes for a draft: `null` restores the setting's default. */
function wire(field: SettingField, draft: Draft): unknown {
  if (field.type === "textList" && isList(draft)) {
    return draft.map((line) => line.trim()).filter((line) => line !== "");
  }
  if (typeof draft === "string" && draft === "" && field.type !== "longText") {
    return null;
  }
  return draft;
}

/**
 * A plugin's settings as a form drawn from the fields it declares, with no code of the plugin's:
 * each labelled from its catalogue, of the kind its type says. A role or channel field offers
 * `communityId`'s own. A secret is never shown; it is replaced by typing a new one. Saving sends
 * only what changed, which `onSave` lays over the settings.
 */
export function SettingsForm({
  plugin,
  fields,
  values,
  secretsSet,
  communityId,
  onSave,
}: {
  plugin: PluginInfo;
  fields: readonly SettingField[];
  values: Record<string, unknown>;
  secretsSet: readonly string[];
  communityId?: string;
  onSave: (patch: Record<string, unknown>) => Promise<void>;
}) {
  const m = useMessages();
  const start = () => Object.fromEntries(fields.map((f) => [f.name, initial(f, values)]));
  const [drafts, setDrafts] = useState<Record<string, Draft>>(start);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  if (fields.length === 0) {
    return <p className={hintClass}>{m.plugins.noSettings}</p>;
  }
  const original = start();
  const changed = fields.filter((f) => !same(drafts[f.name] ?? null, original[f.name] ?? null));

  function set(name: string, draft: Draft) {
    setSaved(false);
    setDrafts((current) => ({ ...current, [name]: draft }));
  }

  function save() {
    if (pending || changed.length === 0) {
      return;
    }
    setPending(true);
    setError(null);
    const patch = Object.fromEntries(changed.map((f) => [f.name, wire(f, drafts[f.name] ?? null)]));
    onSave(patch).then(
      () => {
        setPending(false);
        setSaved(true);
      },
      (e: unknown) => {
        setPending(false);
        setError(problemText(e));
      },
    );
  }

  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        save();
      }}
      className="flex flex-col gap-3"
    >
      {fields.map((field) => (
        <Field
          key={field.name}
          plugin={plugin}
          field={field}
          draft={drafts[field.name] ?? null}
          secretSet={secretsSet.includes(field.name)}
          communityId={communityId}
          onChange={(draft) => {
            set(field.name, draft);
          }}
        />
      ))}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <div className="flex items-center gap-3">
        <Button
          type="submit"
          isDisabled={pending || changed.length === 0}
          className={primaryButtonClass}
        >
          {m.plugins.save}
        </Button>
        {saved && changed.length === 0 && (
          <span role="status" className={hintClass}>
            {m.plugins.saved}
          </span>
        )}
      </div>
    </Form>
  );
}

function Field({
  plugin,
  field,
  draft,
  secretSet,
  communityId,
  onChange,
}: {
  plugin: PluginInfo;
  field: SettingField;
  draft: Draft;
  secretSet: boolean;
  communityId: string | undefined;
  onChange: (draft: Draft) => void;
}) {
  const m = useMessages();
  const label = pluginKey(plugin, field.label);
  const description = field.description == null ? undefined : pluginKey(plugin, field.description);
  const help =
    description === undefined ? null : (
      <Text slot="description" className="text-xs text-ink-muted">
        {description}
      </Text>
    );
  switch (field.type) {
    case "boolean":
      return (
        <ChoiceCheckbox
          isSelected={draft === true}
          onChange={onChange}
          label={label}
          {...(description === undefined ? {} : { hint: description })}
        />
      );
    case "integer":
      return (
        <NumberField
          value={typeof draft === "number" ? draft : Number.NaN}
          onChange={(value) => {
            onChange(Number.isNaN(value) ? null : value);
          }}
          {...(field.min == null ? {} : { minValue: field.min })}
          {...(field.max == null ? {} : { maxValue: field.max })}
          isRequired={field.required === true}
          className={fieldClass + " max-w-48"}
        >
          <Label className={labelClass}>{label}</Label>
          <Input className={inputClass} />
          {help}
        </NumberField>
      );
    case "text":
      return (
        <TextField
          value={typeof draft === "string" ? draft : ""}
          onChange={onChange}
          type={field.secret === true ? "password" : "text"}
          {...(field.maxLength == null ? {} : { maxLength: field.maxLength })}
          className={fieldClass}
        >
          <Label className={labelClass}>{label}</Label>
          <Input
            className={inputClass}
            autoComplete="off"
            {...(field.secret === true && secretSet ? { placeholder: m.plugins.secretSet } : {})}
          />
          {help}
        </TextField>
      );
    case "longText":
    case "textList":
      return (
        <TextField
          value={isList(draft) ? draft.join("\n") : typeof draft === "string" ? draft : ""}
          onChange={(text) => {
            onChange(field.type === "textList" ? text.split("\n") : text);
          }}
          className={fieldClass}
        >
          <Label className={labelClass}>{label}</Label>
          <TextArea rows={4} className={inputClass + " resize-y"} />
          {help ??
            (field.type === "textList" ? (
              <Text slot="description" className="text-xs text-ink-muted">
                {m.plugins.listHint}
              </Text>
            ) : null)}
        </TextField>
      );
    case "choice":
      return (
        <OneOf
          label={label}
          help={help}
          value={typeof draft === "string" ? draft : NONE}
          options={field.options.map((o) => ({ id: o.value, name: pluginKey(plugin, o.label) }))}
          onChange={onChange}
        />
      );
    case "role":
    case "roleList":
    case "channel":
    case "channelList":
      return communityId === undefined ? null : (
        <OfCommunity
          field={field}
          label={label}
          help={help}
          draft={draft}
          communityId={communityId}
          onChange={onChange}
        />
      );
  }
}

/** One of `options`, or none. */
function OneOf({
  label,
  help,
  value,
  options,
  onChange,
}: {
  label: string;
  help: ReactNode;
  value: string;
  options: readonly { id: string; name: string }[];
  onChange: (draft: Draft) => void;
}) {
  const m = useMessages();
  return (
    <Select
      value={value}
      onChange={(key) => {
        onChange(key === NONE ? null : String(key));
      }}
      className={fieldClass + " max-w-sm"}
    >
      <Label className={labelClass}>{label}</Label>
      <Button className={selectButtonClass + " py-2"}>
        <SelectValue />
        <CaretDownIcon size={14} aria-hidden="true" className="text-ink-muted" />
      </Button>
      {help}
      <Popover className={selectPopoverClass}>
        <ListBox>
          <ListBoxItem id={NONE} className={optionClass}>
            {m.plugins.noChoice}
          </ListBoxItem>
          {options.map((o) => (
            <ListBoxItem key={o.id} id={o.id} textValue={o.name} className={optionClass}>
              {o.name}
            </ListBoxItem>
          ))}
        </ListBox>
      </Popover>
    </Select>
  );
}

/** A setting naming the community's own roles or channels. */
function OfCommunity({
  field,
  label,
  help,
  draft,
  communityId,
  onChange,
}: {
  field: SettingField;
  label: string;
  help: ReactNode;
  draft: Draft;
  communityId: string;
  onChange: (draft: Draft) => void;
}) {
  const roles = useRoles(communityId);
  const channels = useChannels(communityId);
  const roleField = field.type === "role" || field.type === "roleList";
  const options = roleField
    ? roles.filter((r) => !r.everyone && r.bot == null).map((r) => ({ id: r.id, name: r.name }))
    : channels.filter((c) => c.ty === "text").map((c) => ({ id: c.id, name: `#${c.name}` }));
  if (field.type === "role" || field.type === "channel") {
    return (
      <OneOf
        label={label}
        help={help}
        value={typeof draft === "string" ? draft : NONE}
        options={options}
        onChange={onChange}
      />
    );
  }
  const chosen = new Set(isList(draft) ? draft : []);
  return (
    <fieldset className="flex flex-col gap-2">
      <legend className={labelClass + " mb-1"}>{label}</legend>
      {help}
      <div className="grid gap-2 sm:grid-cols-2">
        {options.map((o) => (
          <ChoiceCheckbox
            key={o.id}
            label={o.name}
            isSelected={chosen.has(o.id)}
            onChange={(selected) => {
              const next = new Set(chosen);
              if (selected) {
                next.add(o.id);
              } else {
                next.delete(o.id);
              }
              onChange(options.filter((x) => next.has(x.id)).map((x) => x.id));
            }}
          />
        ))}
      </div>
    </fieldset>
  );
}
