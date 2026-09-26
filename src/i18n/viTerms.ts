/**
 * Vietnamese for the English text the Rust core emits (metric titles and labels, unit words, notes,
 * error text). The core keeps English source strings stable; anything unknown stays in English.
 */

const EXACT: Readonly<Record<string, string>> = {
  Session: "Phiên",
  Weekly: "Tuần",
  Monthly: "Tháng",
  Daily: "Ngày",
  Week: "Tuần",
  Month: "Tháng",
  Today: "Hôm nay",
  Yesterday: "Hôm qua",
  "Last 30 Days": "30 ngày qua",
  "Last 4 Weeks": "4 tuần qua",
  "Usage Trend": "Xu hướng sử dụng",
  "Extra Usage": "Dùng thêm",
  "Extra usage spent": "Dùng thêm đã chi",
  "Rate Limit Resets": "Lượt đặt lại hạn mức",
  Credits: "Tín dụng",
  "Total Usage": "Tổng mức dùng",
  "Total usage": "Tổng mức dùng",
  "Cursor Models": "Mô hình Cursor",
  "Other Models": "Mô hình khác",
  "On-Demand": "Theo nhu cầu",
  "On Demand": "Theo nhu cầu",
  Requests: "Yêu cầu",
  "Premium Requests": "Yêu cầu cao cấp",
  "Input Tokens": "Token đầu vào",
  "Output Tokens": "Token đầu ra",
  "Cached Input Tokens": "Token đầu vào từ cache",
  "Cache Write Tokens": "Token ghi vào cache",
  "Pay As You Go": "Trả theo mức dùng",
  "Pay-As-You-Go": "Trả theo mức dùng",
  Balance: "Số dư",
  Status: "Trạng thái",
  "Web Searches": "Lượt tìm kiếm web",
  "Key Limit": "Hạn mức khóa API",
  "No usage data": "Không có dữ liệu sử dụng",
  Disabled: "Đã tắt",
  Unlimited: "Không giới hạn",
  Other: "Khác",
  tokens: "token",
  token: "token",
  credits: "tín dụng",
  credit: "tín dụng",
  available: "khả dụng",
  requests: "yêu cầu",
  resets: "lượt",
  searches: "lượt tìm",
  spent: "đã chi",
  left: "còn lại",
  used: "đã dùng",
  limit: "hạn mức",
  budget: "ngân sách",
  "Estimated locally, so it may be off": "Ước tính trên máy nên có thể sai lệch",
  "From your usage history (estimated)": "Theo lịch sử sử dụng của bạn (ước tính)",
  "Not logged in": "Chưa đăng nhập",
  "Refresh failed": "Làm mới thất bại",
  "Claude Local Usage": "Claude · Trên máy này",
  "Machine-local history": "Lịch sử trên máy",
  "Codex Local Usage": "Codex · Trên máy này",
  Spark: "Spark",
  "Cannot connect to the usage service.": "Không kết nối được dịch vụ hạn mức.",
  "The usage service returned an invalid response.": "Dịch vụ hạn mức trả về dữ liệu không hợp lệ.",
  "Cannot read local credentials.": "Không đọc được thông tin đăng nhập trên máy.",
  "Usage limits require a ChatGPT login; API keys do not provide subscription limits.":
    "Hạn mức cần đăng nhập bằng tài khoản ChatGPT; khóa API không có hạn mức gói đăng ký.",
  "Sign in with claude again to grant access to live usage.": "Hãy đăng nhập lại claude để cấp quyền xem hạn mức trực tiếp.",
  "This account needs to be connected again.": "Tài khoản này cần được kết nối lại.",
  "The login service returned an invalid token response.": "Dịch vụ đăng nhập trả về phản hồi mã không hợp lệ.",
  "The login service returned an invalid token.": "Dịch vụ đăng nhập trả về mã không hợp lệ.",
  "The login service omitted the session expiry.": "Dịch vụ đăng nhập không cho biết thời hạn phiên.",
  "The request timed out.": "Yêu cầu quá thời gian chờ.",
  "Unsupported account provider.": "Nhà cung cấp tài khoản không được hỗ trợ.",
  "Cannot start the local login callback.": "Không khởi động được bước nhận kết quả đăng nhập trên máy.",
  "The authorization endpoint is invalid.": "Địa chỉ cấp quyền không hợp lệ.",
  "This login was cancelled.": "Lần đăng nhập này đã bị hủy.",
  "The login was cancelled.": "Đăng nhập đã bị hủy.",
  "Paste the complete code from the browser, including its state suffix.": "Hãy dán đầy đủ mã từ trình duyệt, gồm cả phần hậu tố trạng thái.",
  "Paste the complete code, including the state suffix.": "Hãy dán đầy đủ mã, gồm cả phần hậu tố trạng thái.",
  "Complete the browser login using its local callback.": "Hãy hoàn tất đăng nhập trong trình duyệt để ứng dụng nhận kết quả.",
  "The login callback is unavailable.": "Không nhận được kết quả đăng nhập.",
  "The login service returned invalid credentials.": "Dịch vụ đăng nhập trả về thông tin không hợp lệ.",
  "This login did not grant an independent renewable session.": "Lần đăng nhập này không cấp phiên có thể tự gia hạn.",
  "The signed-in account could not be verified.": "Không xác minh được tài khoản vừa đăng nhập.",
  "The account profile is invalid.": "Hồ sơ tài khoản không hợp lệ.",
  "The callback URL is invalid.": "Địa chỉ trả về không hợp lệ.",
  "The callback belongs to a different login endpoint.": "Địa chỉ trả về thuộc một lần đăng nhập khác.",
  "The browser login was not authorized.": "Đăng nhập trong trình duyệt chưa được cho phép.",
  "The callback is missing its code or state.": "Địa chỉ trả về thiếu mã hoặc trạng thái.",
  "A complete callback URL is required.": "Cần địa chỉ trả về đầy đủ.",
  "The callback state does not match this login.": "Trạng thái trả về không khớp với lần đăng nhập này.",
  "The authorization code is invalid.": "Mã xác thực không hợp lệ.",
  "The local login callback stopped.": "Bước nhận kết quả đăng nhập đã dừng.",
  "This callback does not match an active Quota Control login.": "Kết quả này không khớp với lần đăng nhập Quota Control nào đang chờ.",
  "Login flow expired or does not exist": "Lần đăng nhập đã hết hạn hoặc không tồn tại",
  "This login is no longer active. Start again.": "Lần đăng nhập này không còn hiệu lực. Hãy bắt đầu lại.",
  "This login has expired. Start again.": "Lần đăng nhập đã hết hạn. Hãy bắt đầu lại.",
  "Chat session does not exist": "Phiên chat không còn tồn tại",
  "A stable account identity is unavailable. Connect this account through the browser.":
    "Không xác định được tài khoản. Hãy kết nối tài khoản này qua trình duyệt.",
  "Cannot access the saved account. Try connecting it again.": "Không truy cập được tài khoản đã lưu. Hãy thử kết nối lại.",
  "Cannot renew this account session. Try again later.": "Không gia hạn được phiên của tài khoản này. Hãy thử lại sau.",
  "Cannot verify the signed-in account. Try connecting again.": "Không xác minh được tài khoản vừa đăng nhập. Hãy thử kết nối lại.",
  "Claude account metadata is unavailable. Connect through the browser.": "Không có thông tin tài khoản Claude. Hãy kết nối qua trình duyệt.",
  "Local credentials are invalid. Sign in again.": "Thông tin đăng nhập trên máy không hợp lệ. Hãy đăng nhập lại.",
  "Saved account credentials are unavailable. Reconnect this account.":
    "Không còn thông tin đăng nhập đã lưu của tài khoản này. Hãy kết nối lại.",
  "The ChatGPT user identity is unavailable. Connect this account through the browser.":
    "Không xác định được người dùng ChatGPT. Hãy kết nối tài khoản này qua trình duyệt.",
  "The ChatGPT workspace identity is unavailable. Connect this account through the browser.":
    "Không xác định được không gian làm việc ChatGPT. Hãy kết nối tài khoản này qua trình duyệt.",
  "The login callback ports are in use. Finish the other login and try again.":
    "Cổng nhận kết quả đăng nhập đang bận. Hãy hoàn tất lần đăng nhập kia rồi thử lại.",
  "The login could not be completed. Start a new login and try again.": "Không hoàn tất được lần đăng nhập. Hãy bắt đầu lại.",
  "The login service rejected this code. Start a new login.": "Dịch vụ đăng nhập từ chối mã này. Hãy đăng nhập lại từ đầu.",
  "The renewed session belongs to a different account. Reconnect this account.":
    "Phiên vừa gia hạn thuộc một tài khoản khác. Hãy kết nối lại tài khoản này.",
  "This account session could not be renewed. Reconnect it if the problem persists.":
    "Chưa gia hạn được phiên của tài khoản này. Nếu lỗi còn lặp lại, hãy kết nối lại.",
  "Too many pending logins. Cancel an existing login first.": "Có quá nhiều lần đăng nhập đang chờ. Hãy hủy bớt một lần trước.",
  "Usage updates are rate limited. Waiting before retrying.": "Đang bị giới hạn số lần cập nhật hạn mức. Sẽ chờ rồi thử lại.",
  Sonnet: "Sonnet",
  Fable: "Fable",
  "Local usage history is incomplete: some files could not be read or scan limits were reached.":
    "Lịch sử sử dụng trên máy chưa đầy đủ: một số tệp không đọc được hoặc đã chạm giới hạn quét.",
  "Some malformed, truncated or oversized local log records were skipped.": "Đã bỏ qua một số bản ghi nhật ký bị hỏng, bị cắt hoặc quá lớn.",
  "Some local usage has no supported model price; affected cost totals are unavailable.":
    "Một số mức dùng trên máy chưa có giá mô hình; tổng chi phí liên quan chưa tính được.",
  "Local history scanner failed.": "Quét lịch sử trên máy thất bại.",
};

const LOCAL_SOURCE_NOTE =
  /^(Today\. )?All local sessions on this machine; not account-scoped\. Input includes cached tokens; output is model-generated\. Costs estimate API-equivalent usage using bundled (\d{4})-(\d{2})-(\d{2}) prices, not subscription charges\.$/;

const PATTERNS: ReadonlyArray<readonly [RegExp, (match: RegExpExecArray) => string]> = [
  [
    LOCAL_SOURCE_NOTE,
    (m) =>
      `${m[1] ? "Hôm nay. " : ""}Mọi phiên trên máy này, không theo từng tài khoản. Token đầu vào gồm cả phần lấy từ cache; token đầu ra do mô hình sinh ra. Chi phí là ước tính theo giá API (bảng giá ngày ${Number(m[4])}/${Number(m[3])}/${m[2]}), không phải phí gói đăng ký.`,
  ],
  [/^From your (.+) usage history \(estimated\)$/, (m) => `Theo lịch sử sử dụng ${m[1]} của bạn (ước tính)`],
  [/^From your (.+) usage history\.?$/, (m) => `Theo lịch sử sử dụng ${m[1]} của bạn.`],
  [/^Refresh timed out after (\d+)s$/, (m) => `Quá thời gian làm mới (${m[1]} giây)`],
  [/^Session expired\. Sign in with (.+) again\.$/, (m) => `Phiên đăng nhập đã hết hạn. Hãy đăng nhập lại ${m[1]}.`],
  [/^Sign in with (.+) to load usage\.$/, (m) => `Hãy đăng nhập ${m[1]} để xem hạn mức.`],
  [/^The (.+) CLI on this computer is now signed in to another account\.$/, (m) => `${m[1]} CLI trên máy này đã chuyển sang tài khoản khác.`],
  [/^Chat session was saved, but its window could not open: (.+)$/, (m) => `Đã lưu phiên chat nhưng không mở được cửa sổ: ${m[1]}`],
  [/^(.+) Weekly$/, (m) => `${m[1]} (tuần)`],
  [/^(.+) Monthly$/, (m) => `${m[1]} (tháng)`],
];

export function viTerm(text: string): string | undefined {
  const exact = EXACT[text];
  if (exact !== undefined) return exact;
  for (const [pattern, render] of PATTERNS) {
    const match = pattern.exec(text);
    if (match) return render(match);
  }
  return undefined;
}
