import { CaretRightIcon, MagnifyingGlassIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useDeferredValue, useEffect, useId, useMemo, useState } from "react";
import { Button, Input, Label, SearchField } from "react-aria-components";
import { fieldClass, inputClass, labelClass, linkButtonClass } from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";
import { useNumberFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import type {
  AttributedComponent,
  AttributedPackage,
  Attributions,
} from "@/features/about/attributionTypes";

/**
 * The Open Source Attributions page (`/attributions`, signed in or out): every package each
 * part of Aspen ships, by part, with its version, license, and website, and the license texts
 * it carries, shown one package at a time on request. The list (`virtual:attributions`) is a
 * chunk of its own, loaded when the page opens. A filter narrows every part's list by name or
 * license. Signed out, the page leads back to the sign-in screen.
 */
export function AttributionsScreen({ signedOut = false }: { signedOut?: boolean }) {
  const m = useMessages();
  const attributions = useAttributions();
  const [typed, setTyped] = useState("");
  const query = useDeferredValue(typed.trim().toLocaleLowerCase());
  return (
    <main className="h-full min-h-0 min-w-0 flex-1 overflow-y-auto bg-surface">
      <div className="mx-auto flex max-w-3xl flex-col gap-4 px-4 py-6 md:px-6">
        {signedOut && (
          <Link to="/" className={linkButtonClass + " self-start text-sm"}>
            {m.about.backToSignIn}
          </Link>
        )}
        <h1 className="text-xl font-semibold">{m.about.attributions}</h1>
        <p className="text-sm text-ink-muted">{m.about.attributionsIntro}</p>
        {attributions === undefined ? (
          <div aria-busy="true" className="flex flex-col gap-2">
            <LoadingLabel text={m.about.attributionsLoading} />
            {Array.from({ length: 8 }, (_, i) => (
              <Skeleton key={i} className="h-12 w-full" />
            ))}
          </div>
        ) : attributions === null ? (
          <p role="alert" className="text-sm text-danger">
            {m.about.attributionsLoadFailed}
          </p>
        ) : (
          <>
            {attributions.incomplete.length > 0 && (
              <p role="status" className="text-sm text-danger">
                {m.about.attributionsIncomplete}
              </p>
            )}
            <SearchField value={typed} onChange={setTyped} className={fieldClass + " max-w-sm"}>
              <Label className={labelClass}>{m.about.find}</Label>
              <div className="relative">
                <MagnifyingGlassIcon
                  size={16}
                  aria-hidden="true"
                  className="pointer-events-none absolute top-1/2 start-3 -translate-y-1/2 text-ink-muted"
                />
                <Input className={inputClass + " w-full ps-9"} />
              </div>
            </SearchField>
            <Components attributions={attributions} query={query} />
          </>
        )}
      </div>
    </main>
  );
}

/** The list, once loaded; `undefined` while it loads and `null` when it could not be. */
function useAttributions(): Attributions | null | undefined {
  const [attributions, setAttributions] = useState<Attributions | null | undefined>();
  useEffect(() => {
    let current = true;
    import("virtual:attributions").then(
      (module) => {
        if (current) {
          setAttributions(module.default);
        }
      },
      () => {
        if (current) {
          setAttributions(null);
        }
      },
    );
    return () => {
      current = false;
    };
  }, []);
  return attributions;
}

function Components({ attributions, query }: { attributions: Attributions; query: string }) {
  const m = useMessages();
  const shown = useMemo(
    () =>
      attributions.components.map((component) => ({
        component,
        packages:
          query === ""
            ? component.packages
            : component.packages.filter(
                (p) =>
                  p.name.toLocaleLowerCase().includes(query) ||
                  p.license.toLocaleLowerCase().includes(query),
              ),
      })),
    [attributions, query],
  );
  if (shown.every(({ packages }) => packages.length === 0)) {
    return <p className="text-sm text-ink-muted">{m.about.noMatches}</p>;
  }
  return shown.map(({ component, packages }) =>
    packages.length === 0 ? null : (
      <ComponentSection
        key={component.id}
        component={component}
        packages={packages}
        texts={attributions.texts}
      />
    ),
  );
}

function ComponentSection({
  component,
  packages,
  texts,
}: {
  component: AttributedComponent;
  packages: readonly AttributedPackage[];
  texts: readonly string[];
}) {
  const m = useMessages();
  const numbers = useNumberFormat();
  const heading = useId();
  return (
    <section aria-labelledby={heading} className="flex flex-col gap-2">
      <h2 id={heading} className="flex items-baseline gap-2 text-lg font-semibold">
        {m.about.component[component.id]}
        <span className="text-sm font-normal text-ink-muted">
          {format(m.about.packageCount, { count: numbers.format(packages.length) })}
        </span>
      </h2>
      <ul className="flex flex-col divide-y divide-line rounded-lg border border-line bg-surface-raised">
        {packages.map((p) => (
          <PackageRow key={`${p.name}@${p.version ?? ""}`} item={p} texts={texts} />
        ))}
      </ul>
    </section>
  );
}

function PackageRow({ item, texts }: { item: AttributedPackage; texts: readonly string[] }) {
  const m = useMessages();
  const [open, setOpen] = useState(false);
  const panel = useId();
  return (
    <li className="flex flex-col gap-1 px-3 py-2">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <span className="font-medium break-all">{item.name}</span>
        {item.version !== null ? (
          <span className="font-mono text-xs break-all text-ink-muted">{item.version}</span>
        ) : (
          item.bundledIn !== undefined && (
            <span className="text-xs text-ink-muted">
              {format(m.about.bundledIn, { release: item.bundledIn })}
            </span>
          )
        )}
        <span className="text-sm text-ink-muted">
          {m.about.license}: {item.license === "" ? m.about.licenseUndeclared : item.license}
        </span>
        {item.url !== null && /^https?:\/\//.test(item.url) && (
          <a
            href={item.url}
            target="_blank"
            rel="noreferrer"
            className={linkButtonClass + " text-sm"}
          >
            {m.about.website}
          </a>
        )}
      </div>
      {item.chromium === true && <ChromiumNotices />}
      {item.texts.length > 0 && (
        <>
          <Button
            aria-expanded={open}
            {...(open ? { "aria-controls": panel } : {})}
            onPress={() => {
              setOpen(!open);
            }}
            className="flex items-center gap-1 self-start rounded-md text-sm text-ink-muted outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <CaretRightIcon
              size={12}
              aria-hidden="true"
              className={
                "transition-transform rtl:-scale-x-100 " + (open ? "rotate-90 rtl:-rotate-90" : "")
              }
            />
            {m.about.showLicense}
          </Button>
          {/* Only an opened package's texts are drawn: all of them are megabytes. */}
          {open && (
            <div id={panel} className="flex flex-col gap-2">
              {item.standardText && (
                <p className="text-sm text-ink-muted">{m.about.standardText}</p>
              )}
              {item.texts.map((t) => (
                <figure key={t.file} className="flex flex-col gap-1">
                  <figcaption className="font-mono text-xs text-ink-muted">{t.file}</figcaption>
                  <pre
                    dir="auto"
                    className="rounded-md bg-surface-sunken p-3 font-mono text-xs break-words whitespace-pre-wrap"
                  >
                    {texts[t.text]}
                  </pre>
                </figure>
              ))}
            </div>
          )}
        </>
      )}
    </li>
  );
}

/**
 * Where Chromium's notices are: beside the desktop app, which opens them in the system's
 * browser; elsewhere, only said.
 */
function ChromiumNotices() {
  const m = useMessages();
  const [missing, setMissing] = useState(false);
  const notices = window.aspenDesktop?.chromiumNotices;
  return (
    <div className="flex flex-col items-start gap-1.5">
      <p className="text-sm text-ink-muted">{m.about.chromiumNotices}</p>
      {notices !== undefined && (
        <Button
          onPress={() => {
            notices.open().then(
              (opened) => {
                setMissing(!opened);
              },
              () => {
                setMissing(true);
              },
            );
          }}
          className={secondaryButtonClass}
        >
          {m.about.openChromiumNotices}
        </Button>
      )}
      {missing && (
        <p role="alert" className="text-sm text-danger">
          {m.about.chromiumNoticesMissing}
        </p>
      )}
    </div>
  );
}
