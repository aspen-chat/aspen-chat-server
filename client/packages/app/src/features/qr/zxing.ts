import { prepareZXingModule, readBarcodes } from "zxing-wasm/reader";
import wasmUrl from "zxing-wasm/reader/zxing_reader.wasm?url";

// The reader's WebAssembly is served with the app rather than fetched from a CDN, so scanning
// reaches no one but the deployment.
prepareZXingModule({
  overrides: {
    locateFile: (path: string, prefix: string) =>
      path.endsWith(".wasm") ? wasmUrl : prefix + path,
  },
});

/** The text of the QR code in `image`, or `null` when it holds none. */
export async function readQrCode(image: ImageData): Promise<string | null> {
  const [found] = await readBarcodes(image, {
    formats: ["QRCode"],
    maxNumberOfSymbols: 1,
    tryHarder: true,
  });
  return found?.isValid === true ? found.text : null;
}
