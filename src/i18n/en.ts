/** English catalog: the upstream OpenUsage copy, adapted for Windows and Linux. */
import type { Messages, When } from "./messages";

function when(value: When): string {
  switch (value.kind) {
    case "in":
      return value.duration;
    case "today":
      return `today at ${value.time}`;
    case "tomorrow":
      return `tomorrow at ${value.time}`;
    case "on":
      return `${value.date} at ${value.time}`;
    case "soon":
      return "soon";
  }
}

const VERBS = { resets: "Resets", limit: "Limit", resetExpires: "Reset expires" } as const;

export const en: Messages = {
  language: "English",
  format: {
    duration(days, hours, minutes) {
      if (days > 0) return `${days}d ${hours}h`;
      if (hours > 0) return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
      return `${minutes}m`;
    },
    monthDay: (date) => date.toLocaleDateString("en-US", { month: "short", day: "numeric" }),
    when,
    deadline(verb, value) {
      const prefix = VERBS[verb];
      return value.kind === "in" ? `${prefix} in ${value.duration}` : `${prefix} ${when(value)}`;
    },
    expiryListHeader: (mode) => (mode === "relative" ? "Resets expire in:" : "Resets expire:"),
    list(items) {
      if (items.length <= 1) return items[0] ?? "";
      if (items.length === 2) return `${items[0]} and ${items[1]}`;
      return `${items.slice(0, -1).join(", ")}, and ${items[items.length - 1]}`;
    },
  },
  meter: {
    displayMode: (mode) => (mode === "used" ? "Used" : "Left"),
    headline: (value, mode) => `${value} ${mode === "used" ? "used" : "left"}`,
    leftAtReset: (percent) => `~${percent}% left at reset`,
    usedAtReset: (percent) => `~${percent}% used at reset`,
    overLimitAtReset: (percent) => `~${percent}% over limit at reset`,
    fullAtReset: "~100% used at reset",
    limitReached: "Limit reached",
    spare: (percent) => `~${percent}% spare`,
    notStarted: "Not started",
    freshSessionTooltip: "Sessions start after you send your first message.",
    noData: "No data",
    dollarLimit: (amount, noun) => `${amount} ${noun ?? "limit"}`,
    valueWithWord: (value, word) => `${value} ${word}`,
    noUsageInPeriod: "No usage in this period",
    localEstimateNote: "Estimated locally, so it may be off",
    unknownModels: (count) => (count === 1 ? "Unknown model found" : "Unknown models found"),
    outdated: "Outdated",
    lastUpdated: (duration) => `Last updated ${duration} ago`,
    refreshTimedOut: (seconds) => `Refresh timed out after ${seconds}s`,
    errors: {
      not_logged_in: "Not logged in",
      auth_expired: "Login expired",
      auth_invalid: "Login rejected",
      credential_access: "Couldn't read your login",
      network: "Network error",
      decoding: "Unexpected response",
      http_4xx: "Request rejected",
      http_5xx: "Provider server error",
      rate_limited: "Rate limited, will retry",
      not_available: "Not available",
      other: "Refresh failed",
    },
  },
  dashboard: {
    emptyState: "Turn on Customize to choose what to show.",
    welcomeTitle: "Welcome to Quota Control",
    welcomeMessage: "We set you up with the AI tools found on this computer. Add or hide providers any time.",
    openCustomize: "Open Customize",
    dismiss: "Dismiss",
    showMore: "Show more",
    showLess: "Show less",
    refreshing: "Refreshing",
    hide: "Hide",
    hideProvider: (name) => `Hide ${name}`,
    starForTaskbar: "Star for taskbar",
    unstar: "Unstar",
    refreshProvider: (name) => `Refresh ${name}`,
    customizeEllipsis: "Customize…",
    pinLimit: (max) => `Up to ${max} stars per provider`,
    peak: (readout) => `peak ${readout}`,
    tokensReadout: (count) => `${count} tokens`,
    otherModels: "Other",
    inputTokens: "Input",
    outputTokens: "Output",
    cacheReadTokens: "Cache read",
    cacheWriteTokens: "Cache write",
    resetsEmpty: "No resets available",
    resetsUnknownExpiries: (count) => `${count} available, expiry dates unavailable`,
    expiringSoon: "Expiring soon",
    noAccountsTitle: "Connect an Account to See Limits",
    noAccountsMessage:
      "Add a Claude or Codex account to track session and weekly limits. Tokens used on this computer still show in the Tokens tab without signing in.",
    noAccountsShort: "No accounts yet",
    addAccount: "Add Account",
    tabsLabel: "Dashboard view",
    tab: (key) => ({ quota: "Limits", tokens: "Tokens" })[key],
    openChat: (product) => `Open ${product} in the App`,
    trendRange: (days, first, last) => `${days} days, ${first} – ${last}`,
    expiryStatus: (severity) =>
      ({
        normal: "Reset credits expire in more than 7 days",
        warning: "A reset credit expires within 7 days",
        critical: "A reset credit expires within 48 hours",
      })[severity],
    unknownPricingWarning: "This period used a model with unknown pricing",
  },
  totalSpend: {
    metric: (key) => ({ cost: "Cost", costPerMtok: "Cost/MTok", tokens: "Tokens" })[key],
    metricMenuLabel: "Total Spend Metric",
    periodLabel: "Period",
    period: (key) => ({ today: "Today", yesterday: "Yesterday", last30: "30 Days" })[key],
    empty: (key) =>
      ({
        cost: "No cost data for this period",
        costPerMtok: "No cost-per-token data for this period",
        tokens: "No token data for this period",
      })[key],
    onlyIncludes: (names) => `Only includes ${names}.`,
    ringUnit: (key) =>
      ({ dollars: "dollars", perMtok: "MTok", billion: "billion", million: "million", thousand: "thousand", tokens: "tokens" })[key],
    costPerMtok: (amount) => `${amount}/MTok`,
    totalCostAria: (value, count) => `Total cost ${value} across ${count} providers`,
    totalTokensAria: (value, count) => `Total tokens ${value} across ${count} providers`,
    blendedRateAria: (value, count) => `Blended cost per megatoken ${value} across ${count} providers`,
  },
  chrome: {
    appName: "Quota Control",
    identity: (name, version) => `${name} ${version}`,
    updating: "Updating…",
    nextUpdateMinutes: (minutes) => `Next update in ${minutes}m`,
    nextUpdateSeconds: (seconds) => `Next update in ${seconds}s`,
    refreshNow: "Refresh now (Ctrl+R)",
    options: "Options",
    customize: "Customize",
    settings: "Settings",
    about: (name) => `About ${name}`,
    quit: (name) => `Quit ${name}`,
    back: "Back",
    resetProvider: (name) => `Reset ${name}`,
    resetAll: "Reset All Customization",
    resetAllTitle: "Reset All Customization?",
    resetAllMessage:
      "Turns providers back on for the tools you have installed and resets every provider's metrics and order. Are you sure?",
    resetAllConfirm: "Reset All",
    cancel: "Cancel",
    accounts: "Accounts",
    checkForUpdates: "Check for Updates…",
    installUpdate: (version) => `Install Update ${version}…`,
    aboutDescription: "An unofficial Windows and Linux port of OpenUsage by Robin Ebers, released under the MIT license.",
    openRepository: "Open Source Repository",
    close: "Close",
  },
  customize: {
    alwaysVisible: "Always Visible",
    onDemand: "On Demand",
    dragHere: "Drag metrics here",
    metricCount: (count) => (count === 1 ? "1 metric" : `${count} metrics`),
    starred: "Starred for taskbar",
    unstarred: "Removed from taskbar",
    star: "Star for taskbar",
    unstar: "Unstar",
    enable: (name) => `Turn on ${name}`,
    reorder: "Drag to reorder",
    settingsLinkTitle: "Settings",
    settingsLinkSubtitle: "Notifications, appearance and more",
    customizeLinkTitle: "Customize",
    customizeLinkSubtitle: "Choose what's visible and where",
    undo: "Undo (Ctrl+Z)",
  },
  settings: {
    section: (key) =>
      ({
        general: "General",
        appearance: "Appearance",
        usageDisplay: "Usage Display",
        taskbar: "Taskbar",
        notifications: "Notifications",
        updates: "App Updates",
        commandLine: "Command Line",
        advanced: "Advanced",
      })[key],
    language: "Language",
    showTotalSpend: "Show Tokens Tab",
    launchAtLogin: (platform) => (platform === "windows" ? "Launch with Windows" : "Launch at Login"),
    launchAtLoginError: "Couldn't change launch at login.",
    globalShortcut: "Global Shortcut",
    globalShortcutTooltip: "Open Quota Control from anywhere",
    recordShortcut: "Record Shortcut",
    pressShortcut: "Press keys…",
    clearShortcut: "Clear Shortcut",
    shortcutNeedsModifier: (platform) => `Add Ctrl, Alt, Shift or ${platform === "windows" ? "Win" : "Super"} to the key.`,
    shortcutUnsupported: "That key can't be used in a shortcut.",
    shortcutUnavailable: "Couldn't use this shortcut. Another app may already use it.",
    terminalHelper: "Terminal Helper",
    installCli: "Install",
    uninstallCli: "Uninstall",
    cliNote: "Adds a global usagectl command agents can use to monitor limits.",
    cliStatus: (state, location) => {
      switch (state) {
        case "installed":
          return "Installed. Open a new terminal to use it.";
        case "managed":
          return `Installed with the package at ${location ?? "usagectl"}.`;
        case "conflict":
          return `${location ?? "usagectl"} already exists and wasn't installed by Quota Control.`;
        case "unavailable":
          return "This build doesn't include usagectl.";
        default:
          return null;
      }
    },
    cliFailed: "Couldn't change the terminal helper.",
    localApiNote: (url) => `While Quota Control runs, other apps on this computer can read the same limits at ${url}.`,
    theme: "Theme",
    themeOption: (theme) => ({ system: "System", light: "Light", dark: "Dark" })[theme],
    density: "Density",
    densityOption: (density) => (density === "regular" ? "Default" : "Compact"),
    reduceAnimations: "Reduce Animations",
    timeFormat: "Time Format",
    timeFormatOption: (format) => ({ auto: "Auto", "12h": "12-hour", "24h": "24-hour" })[format],
    showUsageAs: "Show Usage As",
    resetTimes: "Reset Times",
    resetTimesOption: (mode) => (mode === "relative" ? "Countdown" : "Exact time"),
    alwaysShowPacing: "Always Show Pacing",
    alwaysShowPacingNote: "Show how you're pacing on every metric with a reset window, not just ones near their limit.",
    taskbarDisplay: "Taskbar Display",
    taskbarDisplayOption: (display) => ({ text: "Usage", bars: "Bars", icon: "App Icon Only" })[display],
    taskbarNote: (display) =>
      ({
        text: "Starred metrics show as a live strip next to the notification area.",
        bars: "The tray icon shows usage bars for starred metrics and updates live; hover it for the numbers.",
        icon: "The taskbar shows only the Quota Control icon; click it to open the popup.",
      })[display],
    notification: (key) => ({ almostOut: "Almost Out", cuttingItClose: "Cutting It Close", willRunOut: "Will Run Out" })[key],
    notificationNote: (key) =>
      ({
        almostOut: "Alerts when a metric crosses under 10% remaining.",
        cuttingItClose: "Alerts when a metric is projected to finish the period with little left.",
        willRunOut: "Alerts when a metric is projected to run out before it resets.",
      })[key],
    notificationsDenied: "Notifications are turned off for Quota Control. Turn them on in system settings.",
    allowNotifications: "Allow Notifications",
    copyLogPath: "Copy Log Path",
    revealLog: (platform) => (platform === "windows" ? "Show in File Explorer" : "Open Log Folder"),
    logActionFailed: "Couldn't complete the log file action.",
    copied: "Copied",
    resetAllSettings: "Reset All Settings…",
    resetAllSettingsTitle: "Reset All Settings?",
    resetAllSettingsMessage:
      "Restores every setting and customization to its default. Connected accounts and saved data are kept. This cannot be undone.",
    resetAllSettingsConfirm: "Reset",
  },
  accounts: {
    connected: "Connected Accounts",
    none: "No accounts yet. Add one below to see your limits.",
    mode: (mode, cliName) =>
      ({
        shared_cli: "Copy of the CLI login · read-only",
        managed_oauth: "Signed in through the browser",
        cli: `Automatic from ${cliName} on this computer`,
      })[mode],
    status: (kind) => ({ ok: "Active", refreshing: "Refreshing…", error: "Error", unknown: "No data yet" })[kind],
    add: "Add Account",
    signInWithGoogle: "Sign In with Google",
    signInNote: (brand) =>
      `Opens the ${brand} sign-in page in Google Chrome. Choose Continue with Google, then allow access. The account connects by itself, with no code to copy.`,
    cliNote: "Claude Code and the Codex CLI signed in on this computer appear here automatically.",
    starting: "Opening the sign-in page…",
    waiting: (brand, browser) =>
      browser === "chrome" ? `Waiting for you to sign in to ${brand} in Google Chrome…` : `Waiting for you to sign in to ${brand} in the browser…`,
    waitingNote: "When you finish, Quota Control comes back with the new account.",
    cancel: "Cancel",
    openSignInPage: "Open Sign-In Page Again",
    connectedNotice: (brand) => `${brand} connected`,
    notConnectedNotice: (brand) => `Couldn't connect ${brand}`,
    loginFailed: (brand, detail) => `Couldn't connect ${brand}: ${detail}`,
    remove: "Remove",
    removeTitle: (label) => `Remove ${label}?`,
    removeMessage: "This account's saved login is deleted from this computer. The CLI login, if any, is not affected.",
    removeConfirm: "Remove Account",
    failed: (detail) => `That didn't work: ${detail}`,
    chats: "In-App Chat Sessions",
    chatsNote: "Open the official Claude or ChatGPT website in its own window. Each session keeps its own sign-in.",
    chatsAuthNote: "Website sign-in is separate from usage sign-in. Google sign-in can be blocked in embedded windows.",
    newChat: (product) => `New ${product} Session`,
    noChats: "No chat sessions yet.",
    open: "Open",
    chatOpenFailed: "Couldn't open the chat session. The saved session is kept.",
    createdOn: (date) => `Created ${date}`,
  },
  strip: {
    tooltipEmpty: "Quota Control — no starred metric has data yet",
  },
  update: {
    availableTitle: "Update Available",
    availableMessage: (name, version) => `${name} ${version} is ready to install. The app reopens when it's done.`,
    install: "Install Update",
    whatsNew: "What's New",
    checking: "Checking for updates…",
    upToDateTitle: "You're Up to Date",
    upToDateMessage: (name, version) => `${name} ${version} is the latest version.`,
    downloading: (version) => `Downloading ${version}…`,
    installing: (version) => `Installing ${version}…`,
    installingNote: (platform) =>
      platform === "windows" ? "The installer appears, then the app reopens on its own." : "The app restarts on its own when it's done.",
    failedTitle: (stage) =>
      ({ check: "Couldn't Check for Updates", download: "Couldn't Download the Update", install: "The Update Didn't Install" })[stage],
    failure: (reason) =>
      ({
        network: "Couldn't reach the release server. Check your connection and try again.",
        release: "The release has no package for this computer, or its details are damaged.",
        signature: "The download doesn't match Quota Control's signature, so it was discarded.",
        permission: "Administrator permission to install the update was not granted.",
        other: "Something went wrong. Try again later.",
      })[reason],
    retry: "Try Again",
    automaticChecks: "Check for Updates Automatically",
    automaticChecksNote: "Checks at launch and every 6 hours. Updates download only when you choose to install.",
    version: (version) => `Version ${version}`,
    checkNow: "Check Now",
    lastChecked: (time) => `Up to date · checked at ${time}.`,
    availableStatus: (version) => `Version ${version} is available.`,
    unsupported: "This build can't update itself (a development build or an unsupported package). Download new versions from the releases page.",
    openReleases: "Open Releases Page",
  },
  notify: {
    title: (provider, metric) => `${provider} · ${metric}`,
    almostOut: (left, reset) => `Only ${left}% of this limit left.${reset ? ` ${reset}.` : ""}`,
    cuttingItClose: (percent) => `At this pace you'll use about ${percent}% of this limit before it resets.`,
    willRunOut: (eta) =>
      eta ? `At this pace you'll hit the limit before it resets (${eta.charAt(0).toLowerCase()}${eta.slice(1)}).` : "At this pace you'll hit the limit before it resets.",
  },
  term: () => undefined,
};
