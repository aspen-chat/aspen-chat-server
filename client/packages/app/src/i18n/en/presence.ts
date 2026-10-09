export const presence = {
  menu: "Your status",
  change: "Change your status: {status}",
  descriptions: {
    online: "Online or away, by whether you're using Aspen",
    away: "Away, even while you're using Aspen",
    doNotDisturb: "No notifications, calls, or unread marks",
    invisible: "Offline to everyone else",
  },
  durations: {
    forever: "Until I change it",
    fifteenMinutes: "For 15 minutes",
    hour: "For 1 hour",
    threeHours: "For 3 hours",
    eightHours: "For 8 hours",
    day: "For 1 day",
    threeDays: "For 3 days",
  },
  until: "Until {time}",
  forGood: "Until you change it",
  failed: "Your status couldn't be changed: {problem}",
  failedOn: "Your status couldn't be changed on {deployment}: {problem}",
} as const;
