export const fonts = {
  heading: "Fonts",
  hint: "Kept on this device only. Add a family's bold and italic files too, or one variable font file, so emphasis is drawn in it.",
  text: "Text",
  code: "Code",
  textDefault: "Inclusive Sans (Aspen's own)",
  codeDefault: "Intel One Mono (Aspen's own)",
  yourFonts: "Your fonts",
  noFonts: "You haven't added any fonts.",
  oneFile: "1 file",
  files: "{count} files",
  add: "Add font files",
  remove: "Remove",
  removeFamily: "Remove {family}",
  woff2:
    "{file} is a WOFF2 file, which Aspen can't read. Add the family's TTF, OTF, or WOFF file instead.",
  collection: "{file} is a font collection (TTC). Add each font's own TTF or OTF file instead.",
  unrecognized: "{file} isn't a font file Aspen can read. Add a TTF, OTF, or WOFF file.",
  unnamed:
    "{file} doesn't name the family it belongs to, so Aspen can't list it. Try another copy of the font.",
  unreadable:
    "{file} couldn't be read from your device. Check that it's still there, then try again.",
  saveFailed:
    "Your fonts couldn't be saved on this device. Check that it has free space and that this browser keeps site data, then try again.",
  readFailed:
    "Your fonts couldn't be read from this device's storage. Check that this browser keeps site data, then reopen Settings.",
} as const;
