/** English catalog: the upstream OpenUsage copy, adapted for Windows and Linux. */
import type { Messages, RestoreDay, When } from "./messages";
import { pricesEn, usageEn } from "./usageEn";

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

function restoreDay(day: RestoreDay): string {
  switch (day.kind) {
    case "today":
      return "today";
    case "tomorrow":
      return "tomorrow";
    case "on":
      return day.date.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" });
  }
}

const VERBS = { resets: "Resets", limit: "Limit", resetExpires: "Reset expires" } as const;

function lowerFirst(text: string): string {
  return text.charAt(0).toLowerCase() + text.slice(1);
}

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
    restoresAt: (time, day) => `Back at ${time} · ${restoreDay(day)}`,
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
    starFor: (bar) => (bar === "menuBar" ? "Star for menu bar" : "Star for taskbar"),
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
    tab: (key) => ({ quota: "Limits", tokens: "Tokens", prices: "Prices", benchmark: "Benchmark", resets: "Resets" })[key],
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
    period: (key) => ({ today: "Today", last30: "30 Days", last365: "1 Year", all: "All" })[key],
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
  usage: usageEn,
  prices: pricesEn,
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
    starred: (bar) => (bar === "menuBar" ? "Starred for menu bar" : "Starred for taskbar"),
    unstarred: (bar) => (bar === "menuBar" ? "Removed from menu bar" : "Removed from taskbar"),
    star: (bar) => (bar === "menuBar" ? "Star for menu bar" : "Star for taskbar"),
    unstar: "Unstar",
    enable: (name) => `Turn on ${name}`,
    reorder: "Drag to reorder",
    settingsLinkTitle: "Settings",
    settingsLinkSubtitle: "Notifications, appearance and more",
    customizeLinkTitle: "Customize",
    customizeLinkSubtitle: "Choose what's visible and where",
    undo: (platform) => (platform === "macos" ? "Undo (⌘Z)" : "Undo (Ctrl+Z)"),
  },
  settings: {
    section: (key) =>
      ({
        general: "General",
        appearance: "Appearance",
        usageDisplay: "Usage Display",
        taskbar: "Taskbar",
        menuBar: "Menu Bar",
        island: "Dynamic Island",
        widget: "Desktop Widget",
        notifications: "Notifications",
        updates: "App Updates",
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
    shortcutNeedsModifier: (platform) =>
      platform === "macos" ? "Add ⌘, ⌥, ⌃ or ⇧ to the key." : `Add Ctrl, Alt, Shift or ${platform === "windows" ? "Win" : "Super"} to the key.`,
    shortcutUnsupported: "That key can't be used in a shortcut.",
    shortcutUnavailable: "Couldn't use this shortcut. Another app may already use it.",
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
    barDisplay: (bar) => (bar === "menuBar" ? "Menu Bar Display" : "Taskbar Display"),
    taskbarDisplayOption: (display) => ({ text: "Usage", bars: "Bars", icon: "App Icon Only" })[display],
    barNote: (display, bar) =>
      bar === "menuBar"
        ? {
            text: "Each account shows its mark and readings live in the menu bar.",
            bars: "The menu bar icon shows usage bars and updates live; hover it for the numbers.",
            icon: "The menu bar shows only the Quota Control icon; click it to open the popup.",
          }[display]
        : {
            text: "Each account shows its mark and readings as a live strip next to the notification area.",
            bars: "The tray icon shows usage bars and updates live; hover it for the numbers.",
            icon: "The taskbar shows only the Quota Control icon; click it to open the popup.",
          }[display],
    dynamicIsland: "Show Dynamic Island",
    dynamicIslandNote:
      "Readings sit either side of the notch, or in a pill in the menu bar on screens without one. Expand it for every account, click to open Quota Control.",
    desktopWidget: "Add a Widget",
    desktopWidgetNote:
      "Right-click the desktop, choose Edit Widgets and search for Quota Control. There are three styles: Details (usage bars and reset times), Rings (percentage gauges) and Compact (one line per metric), each in four sizes.",
    glanceContent: "Content",
    glanceContentOption: (content) => ({ dashboard: "Like the Limits tab", starred: "Starred metrics", custom: "Custom" })[content],
    glanceContentNote: (content) =>
      ({
        dashboard: "Every account and metric the Limits tab shows, in the same order.",
        starred: "Only the metrics starred on the Limits tab.",
        custom: "Only the metrics switched on in the list below.",
      })[content],
    glanceMetrics: "Chosen Metrics",
    glanceMetricsNote: "Switch on the metrics to show. They follow the Limits tab's order.",
    glanceMetricsNone: "No account is enabled.",
    glanceShowAccount: "Show Account Email",
    glanceShowPlan: "Show Plan (Pro, Plus…)",
    glanceShowResets: "Show Reset Times",
    glanceShowProblems: "Show Accounts Needing Attention",
    glanceShowProblemsNote: "An account signed out or without readings keeps a line saying why instead of disappearing.",
    stripValues: "Readings per Account",
    stripValuesOption: (count) => (count === 2 ? "Two, stacked" : "One"),
    islandStyle: "Closed Style",
    islandStyleOption: (style) => ({ percent: "Percentage", ring: "Ring", bar: "Bar" })[style],
    islandWing: (side) => (side === "left" ? "Left of the Notch" : "Right of the Notch"),
    islandWingAuto: "Automatic",
    islandExpandOnHover: "Expand on Hover",
    islandExpandOnHoverNote: "When off, click once to expand and again to open Quota Control.",
    islandAlerts: "Open for Alerts",
    islandAlertsNote: "The island opens for a few seconds when a limit runs low or comes back.",
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
    revealLog: (platform) => ({ windows: "Show in File Explorer", macos: "Show in Finder", linux: "Open Log Folder", other: "Open Log Folder" })[platform],
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
    tooltipEmpty: "Quota Control — no metric has data yet",
  },
  glance: {
    empty: {
      dashboard: "Connect an account in Quota Control to show its limits here.",
      starred: "Star metrics in Quota Control to show them here.",
      custom: "Choose metrics in Quota Control's Settings to show them here.",
    },
    noData: "No data yet",
    more: "more",
    updated: "Updated",
    resetsIn: "Resets in",
    resetting: "Resetting…",
    open: "Click to open Quota Control",
    notRunning: "Open Quota Control to show your limits here.",
    units: { day: "d", hour: "h", minute: "m" },
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
  limitReset: {
    redeem: "Use 1 Reset",
    redeeming: "Using…",
    confirmTitle: "Use 1 Limit Reset?",
    confirmMessage: (expiry) =>
      `Codex restores your limit now: the 5-hour limit, the weekly limit or both, as OpenAI decides. The reset that expires first is used${expiry ? ` (it expires ${expiry})` : ""}. This can't be undone.`,
    confirm: "Use 1 Reset",
    result(result, errors) {
      switch (result.status) {
        case "reset":
          return "Used 1 limit reset. Your Codex limit is back.";
        case "inFlight":
          return "A limit reset is already in progress.";
        case "failed":
          return `Couldn't confirm the reset (${lowerFirst(errors[result.category])}). Pressing again repeats the same request, so it can't use two.`;
        case "rejected":
          switch (result.code) {
            case "no_credit":
              return "No limit resets are left.";
            case "nothing_to_reset":
              return "There is nothing to reset yet, so the reset was kept.";
            case "already_redeemed":
              return "This reset was already used elsewhere.";
            default:
              return `OpenAI refused the reset (${result.code}).`;
          }
      }
    },
    failed: "Couldn't use the limit reset. Try again later.",
  },
  term: () => undefined,
};
