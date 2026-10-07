export const settings = {
  motionSpeed: "Animation speed",
  motionOff: "Off",
  motionNormal: "Normal",
  motionTimes: "{speed}×",
  motionHint:
    "All your devices share this. At the far left, nothing animates. Where a device is set to reduce motion, things fade in place rather than move.",
  zoom: "Zoom",
  zoomKeysHint:
    "Only this device follows this setting. {key} + and {key} − change it too, and {key} 0 puts it back to 100%.",
  zoomOnIos:
    "Aspen's text follows the size set in Settings, Accessibility, Display & Text Size, Larger Text. Display Zoom, under Display & Brightness, makes everything larger.",
  zoomOnAndroid:
    "Aspen follows your phone's Font size and Display size, under Settings, Display (or Accessibility, on some phones).",
  zoomInBrowser:
    "Your browser sets how large Aspen is drawn. Press Ctrl + or Ctrl − (⌘ + or ⌘ − on a Mac), or use its menu.",
  messageTextSize: "Message text size",
  messageTextSizeValue: "{size}px",
  messageTextSizeSample: "Messages will be this size.",
  messageTextSizeHint:
    "All your devices share this. It sizes messages and the message box, on top of the text size set for the whole app.",
  messageSpacing: "Line spacing",
  messageSpacingNormal: "Normal",
  messageSpacingWide: "Wide",
  messageSpacingWider: "Wider",
  messageSpacingSample:
    "Lines of a message sit this far apart, and wrap like this when they run long.",
  messageSpacingSampleParagraph: "A new paragraph starts this far below.",
  messageSpacingHint:
    "All your devices share this. It spaces the lines and paragraphs of messages.",
  accessibility: "Accessibility",
  announceMessages: "Read out new messages",
  announceMessagesHint:
    "Your screen reader says who wrote each message that arrives in the conversation you have open, and what it says. Only this device follows this setting.",
  nameColors: "Colour names by role",
  nameColorsHint:
    "Draws people's names in their roles' colours. Only this device follows this setting.",
  title: "Settings",
  account: "Account",
  audio: "Audio and video",
  appearance: "Appearance",
  language: "Language",
  privacy: "Privacy",
  typingNotices: "Show others when I'm typing",
  typingNoticesHint:
    "People in a conversation see that you're writing a message there. Turned off, they don't, and you still see when they are. Every device and server you use follows this setting.",
  languageLabel: "Show Aspen in",
  languageAutomatic: "Automatic ({language})",
  languageHint:
    "Automatic follows your browser's languages. Your choice follows your account to every device, and servers write their messages in it too.",
  pseudoAccented: "Accented English (for testing)",
  pseudoMirrored: "Mirrored English (for testing right-to-left)",
  microphone: "Microphone",
  speaker: "Speaker",
  notificationOutput: "Notification sounds",
  systemDefault: "System default",
  sameAsVoice: "Same as voice chat",
  missingDevice: "Remembered device (not connected)",
  unnamedMicrophone: "Microphone {index}",
  unnamedSpeaker: "Speaker {index}",
  unnamedCamera: "Camera {index}",
  camera: "Camera",
  microphoneLocked: "Your microphones and speakers are listed once Aspen may use the microphone.",
  allowMicrophone: "Allow microphone",
  microphoneDenied:
    "Microphone access was refused, so microphones and speakers cannot be listed. Allow it in your browser or system settings to choose one.",
  noMicrophone: "No microphone is connected.",
  cameraLocked: "Your cameras are listed once Aspen may use the camera.",
  allowCamera: "Allow camera",
  cameraDenied:
    "Camera access was refused, so cameras cannot be listed. Allow it in your browser or system settings to choose one.",
  noCamera: "No camera is connected.",
  outputUnsupported: "This browser cannot choose a speaker; the system default is used.",
} as const;
