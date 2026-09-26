/** Vietnamese catalog — the default language. Numbers follow vi-VN (see `numbers.ts`). */
import type { Messages, When } from "./messages";
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

const VERBS = {
  resets: { lead: "Đặt lại", soon: "Sắp đặt lại" },
  limit: { lead: "Hết hạn mức", soon: "Sắp hết hạn mức" },
  resetExpires: { lead: "Hết hạn", soon: "Sắp hết hạn" },
} as const;

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
    monthDay: (date) => `${date.getDate()}/${date.getMonth() + 1}`,
    when,
    deadline(verb, value) {
      const phrases = VERBS[verb];
      if (value.kind === "soon") return phrases.soon;
      if (value.kind === "in") return `${phrases.lead} sau ${value.duration}`;
      return `${phrases.lead} lúc ${when(value)}`;
    },
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
    copyScreenshot: (name) => `Sao chép ảnh chụp ${name}`,
    copiedToClipboard: "Đã sao chép vào bộ nhớ tạm",
    hide: "Ẩn",
    hideProvider: (name) => `Ẩn ${name}`,
    starForTaskbar: "Gắn sao lên thanh tác vụ",
    unstar: "Bỏ gắn sao",
    refreshProvider: (name) => `Làm mới ${name}`,
    customizeEllipsis: "Tùy chỉnh…",
    shareScreenshot: "Chia sẻ ảnh chụp",
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
    localUsageTitle: (brand) => `${brand} · Trên máy này`,
    noAccountsTitle: "Kết nối tài khoản để xem hạn mức",
    noAccountsMessage:
      "Thêm tài khoản Claude hoặc Codex để theo dõi hạn mức phiên và hạn mức tuần. Chi phí dùng trên máy này vẫn được tính mà không cần đăng nhập.",
    noAccountsShort: "Chưa có tài khoản",
    addAccount: "Thêm tài khoản",
    openChat: (product) => `Mở ${product} trong ứng dụng`,
    trendRange: (days, first, last) => `${days} ngày, ${first} – ${last}`,
    expiryStatus: (severity) =>
      ({
        normal: "Các lượt đặt lại còn hơn 7 ngày mới hết hạn",
        warning: "Có lượt đặt lại hết hạn trong 7 ngày",
        critical: "Có lượt đặt lại hết hạn trong 48 giờ",
      })[severity],
    unknownPricingWarning: "Khoảng này dùng mô hình chưa rõ giá",
  },
  totalSpend: {
    metric: (key) => ({ cost: "Chi phí", costPerMtok: "Chi phí mỗi triệu token", tokens: "Token" })[key],
    metricMenuLabel: "Chỉ số tổng chi tiêu",
    periodLabel: "Khoảng thời gian",
    period: (key) => ({ today: "Hôm nay", yesterday: "Hôm qua", last30: "30 ngày" })[key],
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
    noEnabledProviders: "Chưa bật nhà cung cấp nào",
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
    shareScreenshot: "Chia sẻ ảnh chụp",
    copiedToClipboard: "Đã sao chép vào bộ nhớ tạm",
    copyFailed: "Không sao chép được ảnh chụp",
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
    starred: "Đã gắn sao lên thanh tác vụ",
    unstarred: "Đã bỏ khỏi thanh tác vụ",
    star: "Gắn sao lên thanh tác vụ",
    unstar: "Bỏ gắn sao",
    enable: (name) => `Bật ${name}`,
    reorder: "Kéo để sắp xếp",
    settingsLinkTitle: "Cài đặt",
    settingsLinkSubtitle: "Thông báo, giao diện và các tùy chọn khác",
    customizeLinkTitle: "Tùy chỉnh",
    customizeLinkSubtitle: "Chọn nội dung hiển thị và vị trí",
    undo: "Hoàn tác (Ctrl+Z)",
  },
  settings: {
    section: (key) =>
      ({
        general: "Chung",
        appearance: "Giao diện",
        usageDisplay: "Hiển thị mức dùng",
        taskbar: "Thanh tác vụ",
        notifications: "Thông báo",
        updates: "Cập nhật ứng dụng",
        commandLine: "Dòng lệnh",
        advanced: "Nâng cao",
      })[key],
    language: "Ngôn ngữ",
    showTotalSpend: "Hiện tổng chi tiêu",
    launchAtLogin: (platform) => (platform === "windows" ? "Khởi động cùng Windows" : "Khởi động khi đăng nhập"),
    launchAtLoginError: "Không đổi được chế độ khởi động cùng hệ thống.",
    globalShortcut: "Phím tắt toàn cục",
    globalShortcutTooltip: "Mở Quota Control từ bất kỳ đâu",
    recordShortcut: "Ghi phím tắt",
    pressShortcut: "Nhấn tổ hợp phím…",
    clearShortcut: "Xóa phím tắt",
    shortcutNeedsModifier: (platform) => `Hãy nhấn kèm Ctrl, Alt, Shift hoặc ${platform === "windows" ? "Win" : "Super"}.`,
    shortcutUnsupported: "Không dùng được phím này cho phím tắt.",
    shortcutUnavailable: "Không đặt được phím tắt này. Có thể một ứng dụng khác đang dùng nó.",
    terminalHelper: "Lệnh cho terminal",
    installCli: "Cài đặt",
    uninstallCli: "Gỡ",
    cliNote: "Thêm lệnh usagectl dùng được ở mọi nơi để agent theo dõi hạn mức.",
    cliStatus: (state, location) => {
      switch (state) {
        case "installed":
          return "Đã cài. Mở terminal mới để dùng lệnh.";
        case "managed":
          return `Đã có sẵn cùng gói cài đặt tại ${location ?? "usagectl"}.`;
        case "conflict":
          return `${location ?? "usagectl"} đã tồn tại và không do Quota Control tạo.`;
        case "unavailable":
          return "Bản dựng này không kèm lệnh usagectl.";
        default:
          return null;
      }
    },
    cliFailed: "Không thay đổi được lệnh cho terminal.",
    localApiNote: (url) => `Khi Quota Control đang chạy, ứng dụng khác trên máy này đọc được cùng số liệu hạn mức tại ${url}.`,
    iconStyle: "Kiểu hiển thị",
    iconStyleOption: (style) => (style === "text" ? "Chữ số" : "Thanh"),
    theme: "Chủ đề",
    themeOption: (theme) => ({ system: "Theo hệ thống", light: "Sáng", dark: "Tối" })[theme],
    density: "Mật độ",
    densityOption: (density) => (density === "regular" ? "Mặc định" : "Gọn"),
    reduceAnimations: "Giảm hiệu ứng chuyển động",
    timeFormat: "Định dạng giờ",
    timeFormatOption: (format) => ({ auto: "Tự động", "12h": "12 giờ", "24h": "24 giờ" })[format],
    showUsageAs: "Hiển thị theo",
    resetTimes: "Thời điểm đặt lại",
    resetTimesOption: (mode) => (mode === "relative" ? "Đếm ngược" : "Giờ chính xác"),
    alwaysShowPacing: "Luôn hiện nhịp dùng",
    alwaysShowPacingNote:
      "Hiện dự báo và vạch nhịp đều trên mọi chỉ số có thời điểm đặt lại, không chỉ những chỉ số gần chạm hạn mức.",
    showOnTaskbar: "Hiện số liệu trên thanh tác vụ",
    taskbarNote: (supported) =>
      supported
        ? "Các chỉ số gắn sao hiện ngay trên thanh tác vụ và cập nhật trực tiếp."
        : "Biểu tượng khay hiện thanh mức dùng của các chỉ số gắn sao và cập nhật trực tiếp; di chuột lên biểu tượng để xem số liệu.",
    notification: (key) => ({ almostOut: "Sắp hết", cuttingItClose: "Sát hạn mức", willRunOut: "Sẽ hết trước khi đặt lại" })[key],
    notificationNote: (key) =>
      ({
        almostOut: "Báo khi một chỉ số còn dưới 10%.",
        cuttingItClose: "Báo khi dự báo cuối kỳ chỉ còn lại rất ít.",
        willRunOut: "Báo khi dự báo sẽ hết hạn mức trước thời điểm đặt lại.",
      })[key],
    notificationsDenied: "Thông báo đang bị tắt cho Quota Control. Hãy bật lại trong cài đặt hệ thống.",
    allowNotifications: "Cho phép thông báo",
    copyLogPath: "Sao chép đường dẫn nhật ký",
    revealLog: (platform) => (platform === "windows" ? "Mở trong File Explorer" : "Mở thư mục nhật ký"),
    logActionFailed: "Không thực hiện được thao tác với tệp nhật ký.",
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
    chats: "Phiên chat trong ứng dụng",
    chatsNote: "Mở giao diện chính thức của Claude hoặc ChatGPT trong cửa sổ riêng. Mỗi phiên giữ đăng nhập riêng của nó.",
    chatsAuthNote: "Đăng nhập trang web tách biệt với đăng nhập hạn mức. Đăng nhập bằng Google có thể bị chặn trong cửa sổ nhúng.",
    newChat: (product) => `Phiên ${product} mới`,
    noChats: "Chưa có phiên chat nào.",
    open: "Mở",
    chatOpenFailed: "Không mở được phiên chat. Phiên đã lưu vẫn được giữ nguyên.",
    createdOn: (date) => `Tạo ngày ${date}`,
  },
  strip: {
    tooltipEmpty: "Quota Control — chưa có chỉ số gắn sao nào có dữ liệu",
  },
  update: {
    availableTitle: "Có phiên bản mới",
    availableMessage: (name, version) => `${name} ${version} đã sẵn sàng. Cài xong, ứng dụng tự mở lại.`,
    install: "Cài bản mới",
    whatsNew: "Xem thay đổi",
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
  term: viTerm,
};
