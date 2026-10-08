import { BookmarksSimpleIcon, TrayIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { RailPage } from "@/features/layout/RailPage";
import { PERSONAL_RAIL } from "@/features/layout/paneSizes";
import { useMessages } from "@/i18n/context";

const tabClass =
  "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm whitespace-nowrap text-ink-muted " +
  "outline-none hover:bg-surface-hover hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 " +
  "aria-[current=page]:bg-surface-raised aria-[current=page]:font-medium " +
  "aria-[current=page]:text-accent aria-[current=page]:shadow-sm";

/**
 * The page of what is the reader's own across every deployment they use, the activity feed or
 * their saved messages (`current`), with a rail to go between the two and whatever more the
 * page puts in it (`rail`).
 */
export function PersonalPage({
  current,
  rail,
  children,
}: {
  current: "activity" | "saved";
  rail?: ReactNode;
  children: ReactNode;
}) {
  const m = useMessages();
  return (
    <RailPage
      sizing={PERSONAL_RAIL}
      paneLabel={m.layout.personalRail}
      navLabel={m.activity.personal}
      contentClassName="mx-auto flex max-w-3xl flex-col gap-3 px-2 py-4 md:px-6"
      rail={
        <>
          <h1 className="px-2 text-lg font-semibold">
            {current === "activity" ? m.activity.heading : m.saved.heading}
          </h1>
          <ul className="-mx-1 flex gap-1 overflow-x-auto px-1 md:flex-col">
            <li className="shrink-0">
              <Link
                to="/activity"
                aria-current={current === "activity" ? "page" : undefined}
                className={tabClass}
              >
                <TrayIcon size={18} aria-hidden="true" />
                {m.activity.open}
              </Link>
            </li>
            <li className="shrink-0">
              <Link
                to="/saved"
                aria-current={current === "saved" ? "page" : undefined}
                className={tabClass}
              >
                <BookmarksSimpleIcon size={18} aria-hidden="true" />
                {m.saved.open}
              </Link>
            </li>
          </ul>
          {rail}
        </>
      }
    >
      {children}
    </RailPage>
  );
}
