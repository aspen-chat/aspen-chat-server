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
  ToggleButton,
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
import {
  optionClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useUnsavedChanges } from "@/features/layout/unsavedChanges";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import {
  changedFields,
  clearable,
  initial,
  isList,
  NONE,
  patchOf,
  textDraft,
  type Draft,
} from "./settingsDrafts";

/**
 * A plugin's settings as a form drawn from the fields it declares, with no code of the plugin's:
 * each labelled from its catalogue, of the kind its type says. A role or channel field offers
 * `communityId`'s own. A secret is never shown: typing a new one replaces it, a field left empty
 * leaves it as it is, and one that may be left unset has a control to clear it. Saving sends
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
  // The secrets to clear, by field: an empty secret field says nothing of the value kept.
  const [clearing, setClearing] = useState<ReadonlySet<string>>(new Set());
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const changed = changedFields(fields, drafts, values, clearing);
  useUnsavedChanges(changed.length > 0);
  if (fields.length === 0) {
    return <p className={hintClass}>{m.plugins.noSettings}</p>;
  }

  function set(name: string, draft: Draft) {
    setSaved(false);
    setDrafts((current) => ({ ...current, [name]: draft }));
    // A value typed replaces the one kept, so there is nothing left to clear.
    if (draft !== null) {
      setClearing((current) => without(current, name));
    }
  }

  function setClear(name: string, clear: boolean) {
    setSaved(false);
    setClearing((current) => (clear ? new Set(current).add(name) : without(current, name)));
    if (clear) {
      setDrafts((current) => ({ ...current, [name]: null }));
    }
  }

  function save() {
    if (pending || changed.length === 0) {
      return;
    }
    setPending(true);
    setError(null);
    onSave(patchOf(changed, drafts, clearing)).then(
      () => {
        setPending(false);
        setSaved(true);
        // A secret saved is not read back, so its field starts over, empty.
        setClearing(new Set());
        setDrafts((current) => ({
          ...current,
          ...Object.fromEntries(fields.flatMap((f) => (f.secret === true ? [[f.name, null]] : []))),
        }));
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
          clearing={clearing.has(field.name)}
          communityId={communityId}
          onChange={(draft) => {
            set(field.name, draft);
          }}
          onClear={(clear) => {
            setClear(field.name, clear);
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

/** The control that clears a secret: outlined, and filled while the secret is to be cleared. */
const clearButtonClass =
  secondaryButtonClass +
  " w-fit selected:border-danger selected:bg-danger-soft selected:text-danger";

function without(names: ReadonlySet<string>, name: string): ReadonlySet<string> {
  const rest = new Set(names);
  rest.delete(name);
  return rest;
}

function Field({
  plugin,
  field,
  draft,
  secretSet,
  clearing,
  communityId,
  onChange,
  onClear,
}: {
  plugin: PluginInfo;
  field: SettingField;
  draft: Draft;
  secretSet: boolean;
  /** Whether the secret kept for the field is to be cleared on saving. */
  clearing: boolean;
  communityId: string | undefined;
  onChange: (draft: Draft) => void;
  onClear: (clear: boolean) => void;
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
        <div className={fieldClass}>
          <TextField
            value={typeof draft === "string" ? draft : ""}
            onChange={(text) => {
              onChange(textDraft(text));
            }}
            type={field.secret === true ? "password" : "text"}
            {...(field.maxLength == null ? {} : { maxLength: field.maxLength })}
            className={fieldClass}
          >
            <Label className={labelClass}>{label}</Label>
            <Input
              className={inputClass}
              autoComplete="off"
              {...(field.secret === true && secretSet
                ? { placeholder: clearing ? m.plugins.secretClearing : m.plugins.secretSet }
                : {})}
            />
            {help}
          </TextField>
          {clearable(field, secretSet) && (
            <ToggleButton
              isSelected={clearing}
              onChange={onClear}
              aria-label={format(m.plugins.clearSecretOf, { setting: label })}
              className={clearButtonClass}
            >
              {m.plugins.clearSecret}
            </ToggleButton>
          )}
        </div>
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
