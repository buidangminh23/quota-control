/** Vietnamese catalog — the default language. Numbers follow vi-VN (see `numbers.ts`). */
import { zonedParts } from "@/model/timeZone";
import type { Messages, RestoreDay, When } from "./messages";
import { pricesVi, usageVi } from "./usageVi";
import { viTerm } from "./viTerms";

function when(value: When): string {
  switch (value.kind) {
    case "in":
      return value.duration;
    case "today":
      return `${value.time} hôm nay`;
    case "tomorrow":
      return `${value.time} ngày mai`;
    case "on":
      return `${value.time} ngày ${value.date}`;
    case "soon":
      return "sắp tới";
  }
}

const WEEKDAYS = ["CN", "T2", "T3", "T4", "T5", "T6", "T7"] as const;

function twoDigits(value: number): string {
  return String(value).padStart(2, "0");
}

function restoreDay(day: RestoreDay): string {
  switch (day.kind) {
    case "today":
      return "hôm nay";
    case "tomorrow":
      return "ngày mai";
    case "on": {
      const parts = zonedParts(day.date);
      return `${WEEKDAYS[parts.weekday]} ${twoDigits(parts.day)}/${twoDigits(parts.month)}`;
    }
  }
}

const VERBS = {
  resets: { lead: "Đặt lại", soon: "Sắp đặt lại" },
  limit: { lead: "Hết hạn mức", soon: "Sắp hết hạn mức" },
  resetExpires: { lead: "Hết hạn", soon: "Sắp hết hạn" },
} as const;

function lowerFirst(text: string): string {
  return text.charAt(0).toLocaleLowerCase("vi-VN") + text.slice(1);
}

function capitalize(text: string): string {
  return text.charAt(0).toLocaleUpperCase("vi-VN") + text.slice(1);
}

export const vi: Messages = {
  language: "Tiếng Việt",
  format: {
    duration(days, hours, minutes) {
      if (days > 0) return `${days} ngày ${hours} giờ`;
      if (hours > 0) return minutes > 0 ? `${hours} giờ ${minutes} phút` : `${hours} giờ`;
      return `${minutes} phút`;
    },
    monthDay: (date) => {
      const parts = zonedParts(date);
      return `${parts.day}/${parts.month}`;
    },
    calendarDate: (date) => {
      const parts = zonedParts(date);
      return `${twoDigits(parts.day)}/${twoDigits(parts.month)}/${parts.year}`;
    },
    when,
    deadline(verb, value) {
      const phrases = VERBS[verb];
      if (value.kind === "soon") return phrases.soon;
      if (value.kind === "in") return `${phrases.lead} sau ${value.duration}`;
      return `${phrases.lead} lúc ${when(value)}`;
    },
    restoresAt: (time, day) => `Hồi lại lúc ${time} · ${restoreDay(day)}`,
    timeOnDay: (time, day) => `${time} · ${restoreDay(day)}`,
    expiryListHeader: (mode) => (mode === "relative" ? "Các lượt đặt lại hết hạn sau:" : "Các lượt đặt lại hết hạn lúc:"),
    list: (items) => new Intl.ListFormat("vi", { type: "conjunction" }).format(items),
  },
  meter: {
    displayMode: (mode) => (mode === "used" ? "Đã dùng" : "Còn lại"),
    headline: (value, mode) => (mode === "used" ? `Đã dùng ${value}` : `Còn ${value}`),
    leftAtReset: (percent) => `Còn ~${percent}% khi đặt lại`,
    usedAtReset: (percent) => `Dùng ~${percent}% khi đặt lại`,
    overLimitAtReset: (percent) => `Vượt ~${percent}% hạn mức khi đặt lại`,
    fullAtReset: "Dùng ~100% khi đặt lại",
    limitReached: "Đã hết hạn mức",
    spare: (percent) => `Dư ~${percent}%`,
    notStarted: "Chưa bắt đầu",
    freshSessionTooltip: "Phiên chỉ bắt đầu sau khi bạn gửi tin nhắn đầu tiên.",
    noData: "Không có dữ liệu",
    sessionTitle: (window) => `Phiên ${window}`,
    dollarLimit(amount, noun) {
      if (noun === undefined || noun === "limit") return `Hạn mức ${amount}`;
      return `${capitalize(viTerm(noun) ?? noun)} ${amount}`;
    },
    valueWithWord(value, word) {
      switch (word) {
        case "spent":
          return `Đã chi ${value}`;
        case "left":
          return `Còn ${value}`;
        case "used":
          return `Đã dùng ${value}`;
        default:
          return `${value} ${viTerm(word) ?? word}`;
      }
    },
    noUsageInPeriod: "Không có hoạt động trong khoảng này",
    localEstimateNote: "Ước tính trên máy nên có thể sai lệch",
    unknownModels: () => "Có mô hình chưa rõ giá",
    outdated: "Dữ liệu cũ",
    lastUpdated: (duration) => `Cập nhật lần cuối ${duration} trước`,
    refreshTimedOut: (seconds) => `Quá thời gian làm mới (${seconds} giây)`,
    errors: {
      not_logged_in: "Chưa đăng nhập",
      auth_expired: "Phiên đăng nhập đã hết hạn",
      auth_invalid: "Thông tin đăng nhập bị từ chối",
      credential_access: "Không đọc được thông tin đăng nhập",
      network: "Lỗi mạng",
      decoding: "Phản hồi không đọc được",
      http_4xx: "Yêu cầu bị từ chối",
      http_5xx: "Máy chủ nhà cung cấp đang lỗi",
      rate_limited: "Bị giới hạn tần suất, sẽ thử lại",
      not_available: "Không khả dụng",
      other: "Làm mới thất bại",
    },
  },
  dashboard: {
    emptyState: "Mở Tùy chỉnh để chọn nội dung hiển thị.",
    welcomeTitle: "Chào mừng đến với Quota Control",
    welcomeMessage: "Ứng dụng đã bật sẵn các công cụ AI tìm thấy trên máy này. Bạn có thể thêm hoặc ẩn nhà cung cấp bất cứ lúc nào.",
    openCustomize: "Mở Tùy chỉnh",
    dismiss: "Đóng",
    showMore: "Xem thêm",
    showLess: "Thu gọn",
    refreshing: "Đang làm mới",
    hide: "Ẩn",
    hideProvider: (name) => `Ẩn ${name}`,
    starFor: (bar) => (bar === "menuBar" ? "Gắn sao lên thanh menu" : "Gắn sao lên thanh tác vụ"),
    unstar: "Bỏ gắn sao",
    refreshProvider: (name) => `Làm mới ${name}`,
    customizeEllipsis: "Tùy chỉnh…",
    pinLimit: (max) => `Tối đa ${max} sao cho mỗi nhà cung cấp`,
    peak: (readout) => `cao nhất ${readout}`,
    tokensReadout: (count) => `${count} token`,
    otherModels: "Khác",
    inputTokens: "Đầu vào",
    outputTokens: "Đầu ra",
    cacheReadTokens: "Đọc từ cache",
    cacheWriteTokens: "Ghi vào cache",
    resetsEmpty: "Không có lượt đặt lại nào",
    resetsUnknownExpiries: (count) => `${count} lượt khả dụng, chưa rõ ngày hết hạn`,
    expiringSoon: "Sắp hết hạn",
    noAccountsTitle: "Kết nối tài khoản để xem hạn mức",
    noAccountsMessage:
      "Thêm tài khoản Claude hoặc Codex để theo dõi hạn mức phiên và hạn mức tuần. Token dùng trên máy này vẫn hiện ở tab Token mà không cần đăng nhập.",
    noAccountsShort: "Chưa có tài khoản",
    addAccount: "Thêm tài khoản",
    tabsLabel: "Chế độ xem",
    tab: (key) => ({ quota: "Hạn mức", tokens: "Token", prices: "Bảng giá", benchmark: "Benchmark", resets: "Reset" })[key],
    openChat: (product) => `Mở ${product} trong ứng dụng`,
    trendRange: (days, first, last) => `${days} ngày, ${first} – ${last}`,
    expiryStatus: (severity) =>
      ({
        normal: "Các lượt đặt lại còn hơn 7 ngày mới hết hạn",
        warning: "Có lượt đặt lại hết hạn trong 7 ngày",
        critical: "Có lượt đặt lại hết hạn trong 48 giờ",
      })[severity],
    unknownPricingWarning: "Khoảng này dùng mô hình chưa rõ giá",
    planTermLeft(left, estimated) {
      const about = estimated ? "~" : "";
      switch (left.kind) {
        case "days":
          return `còn ${about}${left.count} ngày`;
        case "hours":
          return `còn ${about}${left.count} giờ`;
        case "minutes":
          return `còn ${about}${left.count} phút`;
        case "due":
          return "đã tới hạn";
      }
    },
    planTermDay: (day, estimated, ended) => (ended ? restoreDay(day) : `tới ${estimated ? "~" : ""}${restoreDay(day)}`),
    planTermStatedNote: (time, offset, checked) =>
      `Gói hết kỳ lúc ${time} · ${offset}. Ngày do ChatGPT ghi trong phiên đăng nhập${checked ? `, kiểm tra lần cuối ${checked}` : ""}. Gói tự gia hạn thì đây là ngày gia hạn.`,
    planTermEndedNote: (time, offset) =>
      `Kỳ gói đã hết lúc ${time} · ${offset}. ChatGPT gửi ngày của kỳ mới khi phiên đăng nhập tự làm mới.`,
    planTermEstimateNote: (time, offset, started) =>
      `Khoảng ${time} · ${offset}, ước tính. Anthropic chỉ cho biết ngày đăng ký gói (${started}), không cho biết ngày gia hạn, nên ngày này tính theo chu kỳ tháng từ ngày đăng ký.`,
  },
  totalSpend: {
    metric: (key) => ({ cost: "Chi phí", costPerMtok: "Chi phí mỗi triệu token", tokens: "Token" })[key],
    metricMenuLabel: "Chỉ số tổng chi tiêu",
    periodLabel: "Khoảng thời gian",
    period: (key) => ({ today: "Hôm nay", last30: "30 ngày", last365: "1 năm", all: "Tất cả" })[key],
    empty: (key) =>
      ({
        cost: "Không có dữ liệu chi phí trong khoảng này",
        costPerMtok: "Không có dữ liệu chi phí theo token trong khoảng này",
        tokens: "Không có dữ liệu token trong khoảng này",
      })[key],
    onlyIncludes: (names) => `Chỉ tính ${names}.`,
    ringUnit: (key) =>
      ({ dollars: "đô la", perMtok: "mỗi triệu token", billion: "tỷ", million: "triệu", thousand: "nghìn", tokens: "token" })[key],
    costPerMtok: (amount) => `${amount}/triệu token`,
    totalCostAria: (value, count) => `Tổng chi phí ${value} từ ${count} nhà cung cấp`,
    totalTokensAria: (value, count) => `Tổng token ${value} từ ${count} nhà cung cấp`,
    blendedRateAria: (value, count) => `Chi phí bình quân mỗi triệu token ${value} từ ${count} nhà cung cấp`,
  },
  usage: usageVi,
  prices: pricesVi,
  chrome: {
    appName: "Quota Control",
    identity: (name, version) => `${name} ${version}`,
    updating: "Đang cập nhật…",
    nextUpdateMinutes: (minutes) => `Cập nhật sau ${minutes} phút`,
    nextUpdateSeconds: (seconds) => `Cập nhật sau ${seconds} giây`,
    refreshNow: "Làm mới ngay (Ctrl+R)",
    options: "Tùy chọn",
    customize: "Tùy chỉnh",
    settings: "Cài đặt",
    about: (name) => `Giới thiệu ${name}`,
    quit: (name) => `Thoát ${name}`,
    back: "Quay lại",
    resetProvider: (name) => `Đặt lại ${name}`,
    resetAll: "Đặt lại toàn bộ tùy chỉnh",
    resetAllTitle: "Đặt lại toàn bộ tùy chỉnh?",
    resetAllMessage:
      "Bật lại các nhà cung cấp cho những công cụ đã cài trên máy, đồng thời đặt lại chỉ số và thứ tự của mọi nhà cung cấp. Bạn chắc chứ?",
    resetAllConfirm: "Đặt lại tất cả",
    cancel: "Hủy",
    accounts: "Tài khoản",
    checkForUpdates: "Kiểm tra phiên bản mới…",
    installUpdate: (version) => `Cài bản mới ${version}…`,
    aboutDescription:
      "Bản chuyển thể không chính thức của OpenUsage (tác giả Robin Ebers) cho Windows và Linux, phát hành theo giấy phép MIT.",
    openRepository: "Mở kho mã nguồn",
    close: "Đóng",
  },
  customize: {
    alwaysVisible: "Luôn hiển thị",
    onDemand: "Khi mở rộng",
    dragHere: "Kéo chỉ số vào đây",
    metricCount: (count) => `${count} chỉ số`,
    starred: (bar) => (bar === "menuBar" ? "Đã gắn sao lên thanh menu" : "Đã gắn sao lên thanh tác vụ"),
    unstarred: (bar) => (bar === "menuBar" ? "Đã bỏ khỏi thanh menu" : "Đã bỏ khỏi thanh tác vụ"),
    star: (bar) => (bar === "menuBar" ? "Gắn sao lên thanh menu" : "Gắn sao lên thanh tác vụ"),
    unstar: "Bỏ gắn sao",
    enable: (name) => `Bật ${name}`,
    reorder: "Kéo để sắp xếp",
    settingsLinkTitle: "Cài đặt",
    settingsLinkSubtitle: "Thông báo, giao diện và các tùy chọn khác",
    customizeLinkTitle: "Tùy chỉnh",
    customizeLinkSubtitle: "Chọn nội dung hiển thị và vị trí",
    undo: (platform) => (platform === "macos" ? "Hoàn tác (⌘Z)" : "Hoàn tác (Ctrl+Z)"),
  },
  settings: {
    section: (key) =>
      ({
        general: "Chung",
        appearance: "Giao diện",
        usageDisplay: "Hiển thị mức dùng",
        taskbar: "Thanh tác vụ",
        menuBar: "Thanh menu",
        island: "Dynamic Island",
        widget: "Widget màn hình",
        notifications: "Thông báo",
        updates: "Cập nhật ứng dụng",
        advanced: "Nâng cao",
      })[key],
    language: "Ngôn ngữ",
    showTotalSpend: "Hiện tab Token",
    launchAtLogin: (platform) => (platform === "windows" ? "Khởi động cùng Windows" : "Khởi động khi đăng nhập"),
    launchAtLoginError: "Không đổi được chế độ khởi động cùng hệ thống.",
    globalShortcut: "Phím tắt toàn cục",
    globalShortcutTooltip: "Mở Quota Control từ bất kỳ đâu",
    recordShortcut: "Ghi phím tắt",
    pressShortcut: "Nhấn tổ hợp phím…",
    clearShortcut: "Xóa phím tắt",
    shortcutNeedsModifier: (platform) =>
      platform === "macos" ? "Hãy nhấn kèm ⌘, ⌥, ⌃ hoặc ⇧." : `Hãy nhấn kèm Ctrl, Alt, Shift hoặc ${platform === "windows" ? "Win" : "Super"}.`,
    shortcutUnsupported: "Không dùng được phím này cho phím tắt.",
    shortcutUnavailable: "Không đặt được phím tắt này. Có thể một ứng dụng khác đang dùng nó.",
    theme: "Chủ đề",
    themeOption: (theme) => ({ system: "Theo hệ thống", light: "Sáng", dark: "Tối" })[theme],
    density: "Mật độ",
    densityOption: (density) => (density === "regular" ? "Mặc định" : "Gọn"),
    reduceAnimations: "Giảm hiệu ứng chuyển động",
    timeFormat: "Định dạng giờ",
    timeZone: "Múi giờ",
    timeZoneValue: (name, offset) => `${name} · ${offset}`,
    timeZoneNote: (zone) => `Tự nhận theo máy (${zone}). Đổi múi giờ trên máy là mọi giờ trong app đổi theo, không cần mở lại.`,
    timeFormatOption: (format) => ({ auto: "Tự động", "12h": "12 giờ", "24h": "24 giờ" })[format],
    showUsageAs: "Hiển thị theo",
    resetTimes: "Thời điểm đặt lại",
    resetTimesOption: (mode) => (mode === "relative" ? "Đếm ngược" : "Giờ chính xác"),
    alwaysShowPacing: "Luôn hiện nhịp dùng",
    alwaysShowPacingNote:
      "Hiện dự báo và vạch nhịp đều trên mọi chỉ số có thời điểm đặt lại, không chỉ những chỉ số gần chạm hạn mức.",
    barDisplay: (bar) => (bar === "menuBar" ? "Hiển thị trên thanh menu" : "Hiển thị trên thanh tác vụ"),
    taskbarDisplayOption: (display) => ({ text: "Số liệu", bars: "Thanh", icon: "Chỉ icon app" })[display],
    barNote: (display, bar) =>
      bar === "menuBar"
        ? {
            text: "Mỗi tài khoản hiện biểu tượng kèm số liệu ngay trên thanh menu và cập nhật trực tiếp.",
            bars: "Biểu tượng trên thanh menu hiện thanh mức dùng và cập nhật trực tiếp; di chuột lên để xem số liệu.",
            icon: "Thanh menu chỉ hiện biểu tượng Quota Control; bấm vào để mở bảng hạn mức.",
          }[display]
        : {
            text: "Mỗi tài khoản hiện biểu tượng kèm số liệu thành dải cạnh khay hệ thống và cập nhật trực tiếp.",
            bars: "Biểu tượng khay hiện thanh mức dùng và cập nhật trực tiếp; di chuột lên biểu tượng để xem số liệu.",
            icon: "Thanh tác vụ chỉ hiện biểu tượng Quota Control; bấm vào để mở bảng hạn mức.",
          }[display],
    dynamicIsland: "Hiện Dynamic Island",
    dynamicIslandNote:
      "Số liệu nằm hai bên tai thỏ, hoặc trong một viên nhộng trên thanh menu ở màn hình không có tai thỏ. Mở rộng để xem đủ các tài khoản, bấm để mở Quota Control.",
    desktopWidget: "Thêm widget",
    desktopWidgetNote:
      "Bấm chuột phải lên màn hình nền, chọn Sửa tiện ích rồi tìm Quota Control. Có ba kiểu: Chi tiết (thanh mức dùng và giờ đặt lại), Vòng tròn (đồng hồ phần trăm) và Gọn (mỗi chỉ số một dòng), mỗi kiểu có bốn cỡ.",
    desktopWidgetKindsNote:
      "Muốn tách riêng thì thêm widget Reset Codex, Lịch reset Codex hoặc Sắp đặt lại; muốn gộp chung thì thêm widget Tổng quan (hạn mức cùng Reset Codex, cỡ lớn nhất có thêm Sắp đặt lại). Nội dung bên dưới áp dụng cho các widget hạn mức, Sắp đặt lại và Tổng quan.",
    glanceContent: "Nội dung",
    glanceContentOption: (content) => ({ dashboard: "Như tab Hạn mức", starred: "Chỉ số gắn sao", custom: "Tự chọn" })[content],
    glanceContentNote: (content) =>
      ({
        dashboard: "Mọi tài khoản và chỉ số đang hiện ở tab Hạn mức, theo đúng thứ tự ở đó.",
        starred: "Chỉ các chỉ số đã gắn sao trong tab Hạn mức.",
        custom: "Chỉ những chỉ số được bật trong danh sách dưới đây.",
      })[content],
    glanceMetrics: "Chỉ số được chọn",
    glanceMetricsNote: "Bật những chỉ số muốn hiện. Thứ tự theo tab Hạn mức.",
    glanceMetricsNone: "Chưa có tài khoản nào đang bật.",
    glanceShowAccount: "Hiện email tài khoản",
    glanceShowPlan: "Hiện gói (Pro, Plus…)",
    glanceShowResets: "Hiện thời gian đặt lại",
    glanceShowProblems: "Hiện tài khoản cần chú ý",
    glanceShowProblemsNote: "Tài khoản hết phiên đăng nhập hoặc chưa có số liệu vẫn có một dòng báo lý do, thay vì biến mất.",
    stripValues: "Số liệu mỗi tài khoản",
    stripValuesOption: (count) => (count === 2 ? "Hai số xếp chồng" : "Một số"),
    islandStyle: "Kiểu thu gọn",
    islandStyleOption: (style) => ({ percent: "Phần trăm", ring: "Vòng tròn", bar: "Thanh" })[style],
    islandWing: (side) => (side === "left" ? "Bên trái tai thỏ" : "Bên phải tai thỏ"),
    islandWingAuto: "Tự động",
    islandWingSpecial: (wing) =>
      ({
        "quota:next": "Hạn mức đặt lại sớm nhất",
        "codex-resets:next": "Reset Codex · Reset free sắp tới",
        "codex-resets:chance-1": "Reset Codex · Khả năng 24 giờ tới",
        "codex-resets:chance-3": "Reset Codex · Khả năng 3 ngày tới",
        "codex-resets:chance-7": "Reset Codex · Khả năng 7 ngày tới",
        "codex-resets:since": "Reset Codex · Thời gian chưa reset",
      })[wing],
    islandLayout: "Cách hiển thị khi mở",
    islandLayoutOption: (layout) => ({ separate: "Tách riêng", combined: "Gộp chung" })[layout],
    islandLayoutNote: (layout) =>
      ({
        separate: "Mỗi lần chỉ hiện một nội dung: hạn mức, Reset Codex hoặc sắp đặt lại.",
        combined: "Hiện nhiều nội dung cùng lúc, từ trên xuống, theo các công tắc bên dưới.",
      })[layout],
    islandSections: "Gộp những phần",
    islandView: "Khi mở island hiện",
    islandViewOption: (view) => ({ quota: "Hạn mức", resets: "Reset Codex", upcoming: "Sắp đặt lại" })[view],
    islandViewNote: (view, trackerOff) =>
      ({
        quota: "Các tài khoản và chỉ số theo mục Nội dung bên dưới.",
        resets: trackerOff
          ? "Reset free sắp tới, khả năng có reset và thời gian từ lần gần nhất. Cần bật tab Reset hoặc thông báo khi Codex reset."
          : "Reset free sắp tới, khả năng có reset và thời gian từ lần gần nhất.",
        upcoming: "Các hạn mức sắp được đặt lại, sớm nhất lên đầu.",
      })[view],
    islandExpandOnHover: "Mở rộng khi rê chuột",
    islandExpandOnHoverNote: "Tắt đi thì bấm một lần để mở rộng, bấm lần nữa để mở Quota Control.",
    islandAlerts: "Tự mở khi có cảnh báo",
    islandAlertsNote: "Island mở ra vài giây khi một hạn mức sắp hết hoặc vừa được đặt lại.",
    notification: (key) => ({ almostOut: "Sắp hết", cuttingItClose: "Sát hạn mức", willRunOut: "Sẽ hết trước khi đặt lại" })[key],
    notificationNote: (key) =>
      ({
        almostOut: "Báo khi một chỉ số còn dưới 10%.",
        cuttingItClose: "Báo khi dự báo cuối kỳ chỉ còn lại rất ít.",
        willRunOut: "Báo khi dự báo sẽ hết hạn mức trước thời điểm đặt lại.",
      })[key],
    notificationsDenied: "Thông báo đang bị tắt cho Quota Control. Hãy bật lại trong cài đặt hệ thống.",
    allowNotifications: "Cho phép thông báo",
    copied: "Đã sao chép",
    resetAllSettings: "Đặt lại toàn bộ cài đặt…",
    resetAllSettingsTitle: "Đặt lại toàn bộ cài đặt?",
    resetAllSettingsMessage:
      "Mọi cài đặt và tùy chỉnh trở về mặc định. Tài khoản đã kết nối và dữ liệu đã lưu vẫn được giữ. Không thể hoàn tác.",
    resetAllSettingsConfirm: "Đặt lại",
  },
  accounts: {
    connected: "Tài khoản đã kết nối",
    none: "Chưa có tài khoản nào. Thêm một tài khoản bên dưới để xem hạn mức.",
    mode: (mode, cliName) =>
      ({
        shared_cli: "Bản sao đăng nhập CLI · chỉ đọc",
        managed_oauth: "Đăng nhập qua trình duyệt",
        cli: `Tự động từ ${cliName} trên máy này`,
      })[mode],
    status: (kind) => ({ ok: "Đang hoạt động", refreshing: "Đang làm mới…", error: "Có lỗi", unknown: "Chưa có dữ liệu" })[kind],
    add: "Thêm tài khoản",
    signInWithGoogle: "Đăng nhập bằng Google",
    signInWithGitHub: "Đăng nhập bằng GitHub",
    serviceSignInNote: (service, method) =>
      method === "google"
        ? `Trang đăng nhập mở trong Google Chrome. Chọn tài khoản Google rồi cho phép truy cập. ${service} tự kết nối, không cần copy mã.`
        : `Trang đăng nhập mở trong Google Chrome. Đăng nhập bằng tài khoản GitHub rồi cho phép truy cập. ${service} tự kết nối.`,
    methodsLabel: "Cách kết nối",
    quickSignIn: (service, method) => `Đăng nhập ${service} bằng ${method}`,
    quickAdd: (service) => `Thêm ${service}`,
    userCodeLabel: "Mã xác nhận",
    copyCode: "Sao chép mã",
    userCodeNote: "Nhập mã này trên trang vừa mở rồi cho phép truy cập.",
    signInNote: (brand) =>
      `Trang đăng nhập ${brand} mở trong Google Chrome. Chọn Continue with Google rồi cho phép truy cập. Tài khoản tự kết nối, không cần copy mã.`,
    cliNote: "Claude Code và Codex CLI đã đăng nhập trên máy này tự hiện ở đây, không cần thêm.",
    starting: "Đang mở trang đăng nhập…",
    waiting: (brand, browser) =>
      browser === "chrome" ? `Đang chờ bạn đăng nhập ${brand} trong Google Chrome…` : `Đang chờ bạn đăng nhập ${brand} trong trình duyệt…`,
    waitingNote: "Đăng nhập xong, Quota Control tự hiện lại với tài khoản mới.",
    cancel: "Hủy",
    openSignInPage: "Mở lại trang đăng nhập",
    connectedNotice: (brand) => `Đã kết nối ${brand}`,
    notConnectedNotice: (brand) => `Chưa kết nối được ${brand}`,
    loginFailed: (brand, detail) => `Chưa kết nối được ${brand}: ${detail}`,
    remove: "Xóa",
    removeTitle: (label) => `Xóa tài khoản ${label}?`,
    removeMessage: "Thông tin đăng nhập đã lưu của tài khoản này sẽ bị xóa khỏi máy. Đăng nhập của CLI (nếu có) không bị ảnh hưởng.",
    removeConfirm: "Xóa tài khoản",
    failed: (detail) => `Không thực hiện được: ${detail}`,
    chatOpenFailed: "Không mở được phiên chat. Phiên đã lưu vẫn được giữ nguyên.",
    serviceSource: (kind, detail) =>
      ({
        login: `Tự động từ ${detail} trên máy này`,
        env: `Key trong biến môi trường ${detail}`,
        key: `API key ••••${detail}`,
        google: "Đăng nhập bằng Google",
        github: "Đăng nhập bằng GitHub",
      })[kind],
    kindGoogle: "Google",
    kindGitHub: "GitHub",
    kindApiKey: "API key",
    kindCookie: "Cookie",
    appLoginNote: (service, app) =>
      `Quota Control đọc ${service} từ đăng nhập ${app} trên máy này. Đăng nhập ${app} là tài khoản tự hiện ở mục Tài khoản đã kết nối, không cần thêm.`,
    alsoAppLogin: (app) => `Hoặc đăng nhập ${app} trên máy này, tài khoản cũng tự hiện.`,
    searchService: "Tìm nhà cung cấp…",
    noServiceMatch: "Không có nhà cung cấp nào khớp.",
    keyLabel: "Tên gợi nhớ (không bắt buộc)",
    keyPlaceholder: "Dán API key",
    pasteValue: (what) => `Dán ${what}`,
    getKey: "Lấy API key",
    openServicePage: (service) => `Mở trang ${service}`,
    saveKey: "Lưu key",
    keySaved: (service) => `Đã thêm ${service}`,
    keyStoredNote: "Key được lưu bảo vệ trên máy này và chỉ gửi tới chính nhà cung cấp đó.",
    keyEnvNote: (variables) => `Hoặc đặt biến môi trường ${variables}, key đó cũng tự hiện ở đây.`,
    cookieNote:
      "Mở trang của nhà cung cấp khi đã đăng nhập, nhấn F12, vào Application → Cookies, chọn cookie có tên ghi ở ô trên và chép phần giá trị.",
    cookieHeaderNote:
      "Mở trang của nhà cung cấp khi đã đăng nhập, nhấn F12, vào tab Network, chọn một yêu cầu tới trang đó và chép nguyên giá trị Cookie trong Request Headers.",
    changeService: "Đổi nhà cung cấp",
    hiddenNote: "Thẻ này ẩn lúc đầu, bật trong Tùy chỉnh khi cần.",
    removeKeyTitle: (label) => `Xóa key ${label}?`,
    removeKeyMessage: "API key đã lưu sẽ bị xóa khỏi máy, thẻ của nó biến mất.",
    removeKeyConfirm: "Xóa key",
    dismissTitle: (label) => `Xóa ${label}?`,
    dismissMessage: (kind, origin, service) =>
      kind === "env"
        ? `Quota Control thôi đọc key trong ${origin}. Biến môi trường vẫn giữ nguyên. Muốn hiện lại, chọn ${service} ở mục Thêm tài khoản.`
        : `Quota Control thôi đọc tài khoản này. Đăng nhập ${origin} trên máy vẫn giữ nguyên. Muốn hiện lại, chọn ${service} ở mục Thêm tài khoản.`,
    restoreDismissed: (count) => (count === 1 ? "Hiện lại tài khoản đã xóa" : `Hiện lại ${count} tài khoản đã xóa`),
  },
  strip: {
    tooltipEmpty: "Quota Control — chưa có chỉ số nào có dữ liệu",
  },
  glance: {
    empty: {
      dashboard: "Kết nối tài khoản trong Quota Control để hiện hạn mức ở đây.",
      starred: "Gắn sao chỉ số trong Quota Control để hiện ở đây.",
      custom: "Chọn chỉ số trong Cài đặt của Quota Control để hiện ở đây.",
    },
    noData: "Chưa có số liệu",
    more: "tài khoản khác",
    updated: "Cập nhật",
    resetsIn: "Đặt lại sau",
    resetting: "Đang đặt lại…",
    open: "Bấm để mở Quota Control",
    notRunning: "Mở Quota Control để hiện hạn mức ở đây.",
    units: { day: " ngày", hour: " giờ", minute: " phút" },
    resetsOff: "Bật tab Reset hoặc thông báo reset trong Quota Control để xem dự báo.",
    upcoming: "Sắp đặt lại",
    upcomingEmpty: "Chưa có hạn mức nào có giờ đặt lại.",
    wingIn: (span) => `sau ${span}`,
    wingSince: (span) => `đã ${span}`,
    calendarMonth: (month) => `Th${month + 1}`,
    sinceReset: "Chưa reset",
  },
  update: {
    availableTitle: "Có phiên bản mới",
    availableMessage: (name, version) => `${name} ${version} đã sẵn sàng. Cài xong, ứng dụng tự mở lại.`,
    install: "Cài bản mới",
    whatsNew: "Xem thay đổi",
    later: "Để sau",
    hide: "Ẩn",
    close: "Đóng",
    updatedTitle: (version) => `Đã cập nhật lên bản ${version}`,
    updatedMessage: (name, from, to) => `${name} đã được cập nhật từ bản ${from} lên ${to}.`,
    checking: "Đang kiểm tra phiên bản mới…",
    upToDateTitle: "Đang dùng bản mới nhất",
    upToDateMessage: (name, version) => `${name} ${version} là phiên bản mới nhất.`,
    downloading: (version) => `Đang tải bản ${version}…`,
    installing: (version) => `Đang cài bản ${version}…`,
    installingNote: (platform) =>
      platform === "windows" ? "Trình cài đặt sẽ hiện ra rồi ứng dụng tự mở lại." : "Ứng dụng tự khởi động lại khi cài xong.",
    failedTitle: (stage) =>
      ({ check: "Không kiểm tra được phiên bản mới", download: "Không tải được bản mới", install: "Chưa cài được bản mới" })[stage],
    failure: (reason) =>
      ({
        network: "Không kết nối được máy chủ phát hành. Hãy kiểm tra mạng rồi thử lại.",
        release: "Bản phát hành chưa có gói cho máy này hoặc thông tin phát hành bị lỗi.",
        signature: "Tệp tải về không khớp chữ ký của Quota Control nên đã bị loại bỏ.",
        permission: "Chưa được cấp quyền quản trị để cài bản mới.",
        other: "Đã có lỗi xảy ra. Hãy thử lại sau.",
      })[reason],
    retry: "Thử lại",
    automaticChecks: "Tự động kiểm tra phiên bản mới",
    automaticChecksNote: "Kiểm tra khi mở ứng dụng rồi 6 giờ một lần. Bản mới chỉ được tải về khi bạn bấm cài.",
    version: (version) => `Phiên bản ${version}`,
    checkNow: "Kiểm tra ngay",
    lastChecked: (time) => `Đang dùng bản mới nhất · kiểm tra lúc ${time}.`,
    availableStatus: (version) => `Đã có bản ${version}.`,
    unsupported: "Bản đang chạy không tự cập nhật được (bản dựng thử hoặc gói cài không hỗ trợ). Hãy tải bản mới trên trang phát hành.",
    openReleases: "Mở trang phát hành",
  },
  notify: {
    title: (provider, metric) => `${provider} · ${metric}`,
    almostOut: (left, reset) => `Chỉ còn ${left}% hạn mức.${reset ? ` ${reset}.` : ""}`,
    cuttingItClose: (percent) => `Với nhịp hiện tại, bạn sẽ dùng khoảng ${percent}% hạn mức trước khi đặt lại.`,
    willRunOut: (eta) =>
      eta ? `Với nhịp hiện tại, bạn sẽ hết hạn mức trước khi đặt lại (${eta.charAt(0).toLocaleLowerCase("vi-VN")}${eta.slice(1)}).` : "Với nhịp hiện tại, bạn sẽ hết hạn mức trước khi đặt lại.",
  },
  limitReset: {
    redeem: "Dùng 1 lượt",
    redeeming: "Đang dùng…",
    confirmTitle: "Dùng 1 lượt đặt lại?",
    confirmMessage: (expiry) =>
      `Codex sẽ hồi hạn mức ngay: giới hạn 5 giờ, giới hạn tuần hoặc cả hai, do OpenAI chọn. App dùng lượt hết hạn sớm nhất${expiry ? ` (hết hạn lúc ${expiry})` : ""}. Dùng rồi không lấy lại được.`,
    confirm: "Dùng 1 lượt",
    result(result, errors) {
      switch (result.status) {
        case "reset":
          return "Đã dùng 1 lượt đặt lại. Hạn mức Codex đã hồi.";
        case "inFlight":
          return "Lượt đặt lại đang được dùng, chờ chút.";
        case "failed":
          return `Chưa xác nhận được lượt đã dùng hay chưa (${lowerFirst(errors[result.category])}). Bấm lại sẽ gửi đúng yêu cầu cũ nên không bị trừ hai lượt.`;
        case "rejected":
          switch (result.code) {
            case "no_credit":
              return "Không còn lượt đặt lại nào.";
            case "nothing_to_reset":
              return "Hạn mức chưa dùng nên chưa cần đặt lại; lượt vẫn còn nguyên.";
            case "already_redeemed":
              return "Lượt này đã được dùng ở nơi khác.";
            default:
              return `OpenAI không cho dùng lượt này (${result.code}).`;
          }
      }
    },
    failed: "Không dùng được lượt đặt lại. Thử lại sau.",
  },
  term: viTerm,
};
