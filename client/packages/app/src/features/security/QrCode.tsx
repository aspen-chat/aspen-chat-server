import encodeQR from "qr";
import { useMemo } from "react";

/**
 * A QR code drawn as SVG squares, dark on a light quiet zone whatever the theme, since scanners
 * expect that contrast.
 */
export function QrCode({
  text,
  label,
  size = 176,
}: {
  text: string;
  label: string;
  size?: number;
}) {
  const cells = useMemo(() => encodeQR(text, "raw", { ecc: "medium", border: 2 }), [text]);
  const extent = cells.length;
  const path = useMemo(() => {
    let d = "";
    cells.forEach((row, y) => {
      row.forEach((dark, x) => {
        if (dark) {
          d += `M${String(x)} ${String(y)}h1v1h-1z`;
        }
      });
    });
    return d;
  }, [cells]);
  return (
    <svg
      role="img"
      aria-label={label}
      width={size}
      height={size}
      viewBox={`0 0 ${String(extent)} ${String(extent)}`}
      shapeRendering="crispEdges"
      className="rounded-md"
    >
      <rect width={extent} height={extent} fill="#ffffff" />
      <path d={path} fill="#000000" />
    </svg>
  );
}
