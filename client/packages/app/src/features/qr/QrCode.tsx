import { CaretDownIcon, DownloadSimpleIcon } from "@phosphor-icons/react";
import { useMemo, type ReactNode } from "react";
import { Button, Menu, MenuItem, MenuTrigger, Popover } from "react-aria-components";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { saveFile } from "@/features/layout/saveFile";
import { useMessages } from "@/i18n/context";
import { MARK_DRAWING, qrDrawing, qrSvgDocument } from "./qrDrawing";

/** How wide a saved PNG is: sharp when printed a few inches across. */
const PNG_PIXELS = 1024;

/**
 * A QR code with Aspen's line mark in the middle (`qrDrawing`), black on white whatever the
 * theme, since scanners expect that contrast. `cover`, when given, blurs the code and lays
 * itself over it: a code that has stopped working shows its way to a new one there.
 */
export function QrCode({
  text,
  label,
  size = 192,
  cover,
}: {
  text: string;
  label: string;
  size?: number;
  cover?: ReactNode;
}) {
  const drawing = useMemo(() => qrDrawing(text), [text]);
  const { extent, modules, mark } = drawing;
  const covered = cover !== undefined;
  return (
    <div className="relative shrink-0 self-center" style={{ width: size, height: size }}>
      <svg
        role="img"
        aria-label={label}
        aria-hidden={covered}
        width={size}
        height={size}
        viewBox={`0 0 ${String(extent)} ${String(extent)}`}
        shapeRendering="crispEdges"
        className={"rounded-md " + (covered ? "blur-md" : "")}
      >
        <rect width={extent} height={extent} fill="#ffffff" />
        <path d={modules} fill="#000000" />
        <svg
          x={mark.x}
          y={mark.y}
          width={mark.width}
          height={mark.height}
          viewBox={MARK_DRAWING.viewBox}
          shapeRendering="geometricPrecision"
          // The mark is the brand file's own markup, made by `scripts/export_icons.py`.
          dangerouslySetInnerHTML={{ __html: MARK_DRAWING.body }}
        />
      </svg>
      {covered && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 p-3 text-center">
          {cover}
        </div>
      )}
    </div>
  );
}

async function pngOf(svg: string): Promise<Blob> {
  const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = PNG_PIXELS;
    canvas.height = PNG_PIXELS;
    const context = canvas.getContext("2d");
    if (context === null) {
      throw new Error("no 2D canvas");
    }
    context.imageSmoothingEnabled = false;
    context.drawImage(image, 0, 0, PNG_PIXELS, PNG_PIXELS);
    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob((blob) => {
        if (blob === null) {
          reject(new Error("the PNG could not be made"));
        } else {
          resolve(blob);
        }
      }, "image/png");
    });
  } finally {
    URL.revokeObjectURL(url);
  }
}

/**
 * Saves a code as the reader picks: a PNG, for chat and print shops, or an SVG, which scales to
 * any size. `fileName` is without its extension.
 */
export function QrDownload({
  text,
  label,
  fileName,
}: {
  text: string;
  label: string;
  fileName: string;
}) {
  const m = useMessages();
  async function save(kind: "png" | "svg") {
    const svg = qrSvgDocument(qrDrawing(text), label, PNG_PIXELS);
    const file = kind === "svg" ? new Blob([svg], { type: "image/svg+xml" }) : await pngOf(svg);
    await saveFile(file, `${fileName}.${kind}`);
  }
  return (
    <MenuTrigger>
      <Button className={secondaryButtonClass + " flex items-center gap-1.5"}>
        <DownloadSimpleIcon size={14} aria-hidden="true" />
        {m.qr.download}
        <CaretDownIcon size={12} aria-hidden="true" />
      </Button>
      <Popover className="rounded-md border border-line bg-surface-raised p-1 shadow-lg">
        <Menu
          className="min-w-40 outline-none"
          onAction={(key) => {
            if (key === "png" || key === "svg") {
              void save(key);
            }
          }}
        >
          <MenuItem id="png" className={menuItemClass}>
            {m.qr.png}
          </MenuItem>
          <MenuItem id="svg" className={menuItemClass}>
            {m.qr.svg}
          </MenuItem>
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}

const menuItemClass =
  "cursor-default rounded px-2 py-1.5 text-sm outline-none focus:bg-surface-hover pointer-coarse:py-2.5";
