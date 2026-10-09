# The screen picker

Screen sharing on the desktop shell always goes through a picker. In Electron, `getDisplayMedia`
works only because the main process answers it in `setDisplayMediaRequestHandler`
(`packages/desktop/src/main/index.ts`).

| Where | Picker |
| --- | --- |
| Wayland | The desktop portal's. |
| macOS 15 and later | The system picker. |
| Everywhere else | `SourcePickerDialog`, below. |

## `SourcePickerDialog`

1. The main process lists every screen and window with a thumbnail.
2. The renderer shows `SourcePickerDialog` (mounted in `RootLayout`).
3. It answers over the `displayPicker` bridge with the chosen source and, on Windows, whether to take
   the system's audio along.
4. Dismissing it answers with nothing, and the share does not start.

See [voice media](../voice/media.md#screen-sharing) for what the share sends.
