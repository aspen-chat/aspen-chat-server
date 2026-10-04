import { ApiProblemError, explain, type ChannelType } from "@aspen/protocol";
import { CaretDownIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useState, type SyntheticEvent } from "react";
import {
  Button,
  FieldError,
  Form,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
  TextField,
} from "react-aria-components";
import {
  useAccess,
  useCategories,
  useCategoryOverrides,
  useRoles,
  useStore,
  useSync,
} from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  presetOverrides,
  type Preset,
  type PresetOption,
} from "@/features/community-settings/accessPresets";
import { PresetChoices } from "@/features/community-settings/PresetChoices";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useDomain, channelLink } from "@/features/messages/links";
import { optionClass, selectPopoverClass } from "@/features/invites/dialog";

const NO_CATEGORY = "none";

const selectButtonClass =
  "flex justify-between rounded-md border border-line bg-surface px-3 py-2 text-start outline-none " +
  "hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * Names a new channel of a fixed type (or kind of a plugin's), files it under a category (a given
 * one, or one chosen here when the community has any), sets who can use it from the start with
 * the plain settings the access dialog leads with, and opens it if it is a text channel or a
 * plugin's, which its creator can see.
 */
export function CreateChannelForm({
  communityId,
  ty,
  pluginType,
  parentCategory,
  onDone,
}: {
  communityId: string;
  ty: ChannelType;
  /** For a channel of a kind a plugin adds, the kind. */
  pluginType?: string;
  /** Files the channel under this category and hides the choice; absent, the form offers one. */
  parentCategory?: string;
  onDone: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const domain = useDomain();
  const categories = useCategories(communityId);
  const store = useStore();
  const roles = useRoles(communityId);
  const access = useAccess(communityId);
  const [category, setCategory] = useState<string>(parentCategory ?? NO_CATEGORY);
  const [preset, setPreset] = useState<Preset>("everyone");
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set());
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const categoryOverrides = useCategoryOverrides(category === NO_CATEGORY ? "" : category);
  const categoryName = categories.find((c) => c.id === category)?.name;
  const everyone = roles.find((r) => r.everyone);
  // Only roles ranked below the creator's highest can be given an override, everyone's among
  // them, so a creator who outranks no role is offered no choice.
  const settable =
    access === null ? [] : roles.filter((r) => !r.everyone && access.outranks(r.position));
  const choosesAccess = everyone !== undefined && access?.outranks(everyone.position) === true;
  const overrides =
    choosesAccess && preset !== "custom" ? presetOverrides(preset, chosen, everyone.id) : [];
  const shutOut =
    access !== null && !explain(access, "viewChannel", categoryOverrides, overrides).allowed;
  const options: readonly PresetOption[] = [
    categoryName === undefined
      ? { key: "everyone", label: m.access.presets.everyone, hint: m.access.presets.everyoneHint }
      : {
          key: "everyone",
          label: m.access.presets.sameAsCategory,
          hint: format(m.access.presets.sameAsCategoryHint, { category: categoryName }),
        },
    { key: "private", label: m.access.presets.private, hint: m.access.presets.privateHint },
    // A voice channel has no posting to withhold.
    ...(ty === "text"
      ? [
          {
            key: "readOnly" as const,
            label: m.access.presets.readOnly,
            hint: m.access.presets.readOnlyHint,
          },
        ]
      : []),
  ];

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = formString(new FormData(event.currentTarget), "name").trim();
    if (name.length === 0) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      const channel = await sync.createChannel(communityId, {
        name,
        ty,
        parentCategory: category === NO_CATEGORY ? null : category,
        overrides,
        ...(pluginType === undefined ? {} : { pluginType }),
      });
      onDone();
      if (
        (channel.ty === "text" || channel.ty === "plugin") &&
        store.channel(channel.id) !== undefined
      ) {
        await navigate(channelLink({ domain, community: communityId }, channel.id));
      }
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <Form
      onSubmit={(e) => {
        void submit(e);
      }}
      className="flex flex-col gap-3"
    >
      <TextField name="name" isRequired autoFocus className={fieldClass}>
        <Label className={labelClass}>{m.channelNameLabel}</Label>
        <Input className={inputClass} />
        <FieldError className="text-sm text-danger" />
      </TextField>
      {parentCategory === undefined && categories.length > 0 && (
        <Select
          value={category}
          onChange={(key) => {
            if (typeof key === "string") {
              setCategory(key);
            }
          }}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.categoryLabel}</Label>
          <Button className={selectButtonClass}>
            <SelectValue />
            <CaretDownIcon size={14} aria-hidden="true" />
          </Button>
          <Popover className={selectPopoverClass}>
            <ListBox className="outline-none">
              <ListBoxItem id={NO_CATEGORY} textValue={m.noCategory} className={optionClass}>
                {m.noCategory}
              </ListBoxItem>
              {categories.map((c) => (
                <ListBoxItem key={c.id} id={c.id} textValue={c.name} className={optionClass}>
                  {c.name}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
      )}
      {choosesAccess && (
        <div className="flex flex-col gap-3">
          <PresetChoices
            options={options}
            preset={preset}
            onPresetChange={setPreset}
            roles={[...settable].reverse()}
            chosen={chosen}
            onChosenChange={setChosen}
          />
          <p className={hintClass}>{m.access.createNote}</p>
        </div>
      )}
      {shutOut && (
        <p role="status" className="text-sm text-danger">
          {m.access.shutOut}
        </p>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
        {pending ? m.creating : m.create}
      </Button>
    </Form>
  );
}
