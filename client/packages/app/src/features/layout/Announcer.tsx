import { useAnnouncements } from "@/features/layout/announce";

/** The live region `announce` speaks through, kept once at the root of the page. */
export function Announcer() {
  const shown = useAnnouncements();
  return (
    <div aria-live="polite" aria-relevant="additions" className="sr-only">
      {shown.map((announcement) => (
        <p key={announcement.id}>{announcement.text}</p>
      ))}
    </div>
  );
}
