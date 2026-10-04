import { CaretDownIcon, PlusIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import {
  Button,
  FileTrigger,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import {
  optionClass,
  planeClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import {
  addFontFiles,
  listFamilies,
  loadFamily,
  removeFamily,
  type FontFamily,
  type RefusedFont,
} from "@/theme/fontLibrary";
import { FONT_ROLES, aliasStack, applyFont, storedFont, type FontRole } from "@/theme/fonts";

/**
 * The files the picker offers: by extension, and by type, under the older names too, for
 * systems that filter by type (Android's pickers among them, through Capacitor).
 */
const ACCEPTED = [
  ".ttf",
  ".otf",
  ".woff",
  "font/ttf",
  "font/otf",
  "font/woff",
  "font/sfnt",
  "application/x-font-ttf",
  "application/x-font-otf",
  "application/font-woff",
  "application/font-sfnt",
  "application/vnd.ms-opentype",
];

/** Aspen's own faces, as `styles.css` names them, for showing the default in itself. */
const BUNDLED: Record<FontRole, string> = {
  text: '"Inclusive Sans Variable", "Noto Sans Variable", sans-serif',
  code: '"Intel One Mono Variable", monospace',
};

/** The key standing for Aspen's own font in a font select; family names are never empty. */
const DEFAULT_KEY = "";

/**
 * The fonts text and code are drawn in on this install, and the font files the user has added
 * to choose from (`theme/fonts.ts`, `theme/fontLibrary.ts`). Each family is listed in itself.
 */
export function FontsSection() {
  const m = useMessages();
  const [families, setFamilies] = useState<readonly FontFamily[]>([]);
  // The alias each family is drawn under, once its files are registered with the document.
  const [aliases, setAliases] = useState<ReadonlyMap<string, string>>(new Map());
  const [chosen, setChosen] = useState<Record<FontRole, string | null>>(() => ({
    text: storedFont("text"),
    code: storedFont("code"),
  }));
  const [refused, setRefused] = useState<readonly RefusedFont[]>([]);
  const [error, setError] = useState<string | null>(null);

  // Bumped after the library changes, to read it again.
  const [version, setVersion] = useState(0);

  useEffect(() => {
    let live = true;
    readLibrary().then(
      (library) => {
        if (live) {
          setFamilies(library.families);
          setAliases(library.aliases);
        }
      },
      () => {
        if (live) {
          setError(m.fonts.readFailed);
        }
      },
    );
    return () => {
      live = false;
    };
  }, [version, m]);

  const choose = (role: FontRole, family: string | null) => {
    setChosen((current) => ({ ...current, [role]: family }));
    setError(null);
    applyFont(role, family).then(
      (applied) => {
        if (applied !== undefined) {
          setChosen((current) => ({ ...current, [role]: applied }));
        }
      },
      () => {
        setError(m.fonts.readFailed);
      },
    );
  };

  const add = async (files: readonly File[]) => {
    setError(null);
    try {
      const added = await addFontFiles(files);
      setRefused(added.filter((result): result is RefusedFont => "problem" in result));
    } catch {
      setError(m.fonts.saveFailed);
    }
    setVersion((v) => v + 1);
  };

  const remove = async (family: string) => {
    setError(null);
    try {
      await removeFamily(family);
    } catch {
      setError(m.fonts.saveFailed);
      return;
    }
    for (const role of FONT_ROLES) {
      if (chosen[role] === family) {
        choose(role, null);
      }
    }
    setVersion((v) => v + 1);
  };

  return (
    <section aria-labelledby="settings-fonts" className={planeClass}>
      <div>
        <h3 id="settings-fonts" className="text-sm font-semibold text-ink-muted">
          {m.fonts.heading}
        </h3>
        <p className="text-xs text-ink-faint">{m.fonts.hint}</p>
      </div>
      {FONT_ROLES.map((role) => (
        <FontSelect
          key={role}
          role={role}
          families={families}
          aliases={aliases}
          chosen={chosen[role]}
          onChoose={(family) => {
            choose(role, family);
          }}
        />
      ))}
      <div className="flex flex-col gap-1">
        <h4 id="settings-your-fonts" className="text-sm font-medium">
          {m.fonts.yourFonts}
        </h4>
        {families.length === 0 ? (
          <p className="text-sm text-ink-muted">{m.fonts.noFonts}</p>
        ) : (
          <ul aria-labelledby="settings-your-fonts" className="flex flex-col gap-1">
            {families.map((family) => (
              <li key={family.name} className="flex items-center gap-2">
                <span className="flex min-w-0 flex-1 flex-col">
                  <span
                    className="truncate text-sm"
                    style={fontStyle(aliases.get(family.name), "text")}
                  >
                    {family.name}
                  </span>
                  <span className="text-xs text-ink-muted">
                    {family.faces.length === 1
                      ? m.fonts.oneFile
                      : format(m.fonts.files, { count: String(family.faces.length) })}
                  </span>
                </span>
                <Button
                  aria-label={format(m.fonts.removeFamily, { family: family.name })}
                  onPress={() => {
                    void remove(family.name);
                  }}
                  className={secondaryButtonClass}
                >
                  {m.fonts.remove}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <FileTrigger
        acceptedFileTypes={ACCEPTED}
        allowsMultiple
        onSelect={(files) => {
          if (files !== null && files.length > 0) {
            void add(Array.from(files));
          }
        }}
      >
        <Button className={secondaryButtonClass + " flex items-center gap-1.5 self-start"}>
          <PlusIcon size={16} aria-hidden="true" />
          {m.fonts.add}
        </Button>
      </FileTrigger>
      {(refused.length > 0 || error !== null) && (
        <div role="alert" className="flex flex-col gap-1 text-xs text-danger">
          {error !== null && <p>{error}</p>}
          {refused.map((result, index) => (
            <p key={index}>{format(m.fonts[result.problem], { file: result.fileName })}</p>
          ))}
        </div>
      )}
    </section>
  );
}

/** Every family in the library, and the alias of each the browser could register. */
async function readLibrary(): Promise<{
  families: readonly FontFamily[];
  aliases: ReadonlyMap<string, string>;
}> {
  const families = await listFamilies();
  const loaded = await Promise.all(
    families.map(
      async ({ name }) => [name, await loadFamily(name).catch(() => undefined)] as const,
    ),
  );
  return {
    families,
    aliases: new Map(
      loaded.flatMap(([name, alias]) => (alias === undefined ? [] : [[name, alias]])),
    ),
  };
}

function fontStyle(alias: string | undefined, role: FontRole) {
  return alias === undefined ? {} : { fontFamily: aliasStack(alias, role) };
}

function FontSelect({
  role,
  families,
  aliases,
  chosen,
  onChoose,
}: {
  role: FontRole;
  families: readonly FontFamily[];
  aliases: ReadonlyMap<string, string>;
  chosen: string | null;
  onChoose: (family: string | null) => void;
}) {
  const m = useMessages();
  const options = [
    {
      id: DEFAULT_KEY,
      label: role === "text" ? m.fonts.textDefault : m.fonts.codeDefault,
      style: { fontFamily: BUNDLED[role] },
    },
    ...families.map((family) => ({
      id: family.name,
      label: family.name,
      style: fontStyle(aliases.get(family.name), role),
    })),
  ];
  return (
    <Select
      value={chosen ?? DEFAULT_KEY}
      onChange={(key) => {
        if (typeof key === "string") {
          onChoose(key === DEFAULT_KEY ? null : key);
        }
      }}
      className="flex flex-col gap-1"
    >
      <Label className="text-sm font-medium">{role === "text" ? m.fonts.text : m.fonts.code}</Label>
      <Button className={selectButtonClass}>
        <SelectValue className="truncate" />
        <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
      </Button>
      <Popover className={selectPopoverClass}>
        <ListBox items={options}>
          {(option) => (
            <ListBoxItem
              id={option.id}
              textValue={option.label}
              className={optionClass}
              style={option.style}
            >
              {option.label}
            </ListBoxItem>
          )}
        </ListBox>
      </Popover>
    </Select>
  );
}
