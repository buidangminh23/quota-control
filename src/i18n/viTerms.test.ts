import { viTerm } from "./viTerms";

describe("viTerm", () => {
  it("translates the metric titles, labels and units the service readers emit", () => {
    expect(viTerm("Spend This Month")).toBe("Chi tiêu tháng này");
    expect(viTerm("Weekly limit")).toBe("Hạn mức tuần");
    expect(viTerm("Bonus Balance")).toBe("Số dư thưởng");
    expect(viTerm("predictions")).toBe("lượt dự đoán");
    expect(viTerm("Claude Weekly")).toBe("Claude (tuần)");
  });

  it("translates the service framework's messages whatever service they name", () => {
    expect(viTerm("Cannot connect to OpenRouter. Check the connection and try again.")).toBe(
      "Không kết nối được OpenRouter. Hãy kiểm tra kết nối rồi thử lại.",
    );
    expect(viTerm("Z.ai is rate limiting usage requests. Waiting before retrying.")).toBe(
      "Z.ai đang giới hạn số lần hỏi hạn mức. Sẽ chờ rồi thử lại.",
    );
    expect(viTerm("Groq answered with HTTP 502.")).toBe("Groq trả về lỗi HTTP 502.");
    expect(viTerm("The Gemini CLI login expired. Open Gemini CLI once to renew it.")).toBe(
      "Phiên đăng nhập Gemini CLI đã hết hạn. Hãy mở Gemini CLI một lần để gia hạn.",
    );
    expect(viTerm("This account is no longer signed in to Kiro on this computer.")).toBe(
      "Tài khoản này không còn đăng nhập Kiro trên máy này.",
    );
    expect(viTerm("OPENROUTER_API_KEY is no longer set.")).toBe("Biến môi trường OPENROUTER_API_KEY không còn được đặt.");
    expect(viTerm("The LiteLLM server address must start with https://.")).toBe(
      "Địa chỉ máy chủ LiteLLM phải bắt đầu bằng https://.",
    );
    expect(viTerm("Kilo did not start a sign-in. Try again later.")).toBe("Kilo chưa mở được phiên đăng nhập. Hãy thử lại sau.");
    expect(viTerm("This sign-in expired. Sign in again in Accounts.")).toBe(
      "Phiên đăng nhập này đã hết hạn. Hãy đăng nhập lại trong Tài khoản.",
    );
  });

  it("translates the readers' links, units, key fields and composed row labels", () => {
    expect(viTerm("Dashboard")).toBe("Bảng điều khiển");
    expect(viTerm("neurons")).toBe("neuron");
    expect(viTerm("Session cookie (__Secure-authjs.session-token)")).toBe("cookie phiên (__Secure-authjs.session-token)");
    expect(viTerm("Dashboard (default: cloud.helmcode.com, or cloud.nan.builders)")).toBe(
      "Trang quản lý (mặc định: cloud.helmcode.com, hoặc cloud.nan.builders)",
    );
    expect(viTerm("Agent Daily")).toBe("Agent (ngày)");
    expect(viTerm("Weekly Refill")).toBe("Tuần (nạp lại)");
    expect(viTerm("Key · Monthly · Shared · Hard")).toBe("Key · Tháng · Dùng chung · Chặn khi vượt");
    expect(viTerm("Anthropic Cost")).toBe("Chi phí Anthropic");
    expect(viTerm("Premium Requests")).toBe("Yêu cầu cao cấp");
    expect(viTerm("Model: gpt-5")).toBe("Mô hình: gpt-5");
  });

  it("translates each reader's messages, naming the service they come from", () => {
    expect(viTerm("The Venice API key is missing.")).toBe("Thiếu API key Venice.");
    expect(viTerm("The Perplexity session cookie is missing.")).toBe("Thiếu cookie phiên Perplexity.");
    expect(viTerm("The MiMo session cookie is missing or invalid.")).toBe("Cookie phiên MiMo bị thiếu hoặc không hợp lệ.");
    expect(viTerm("Groq refused this API key. Check it or create a new one.")).toBe(
      "Groq từ chối API key này. Hãy kiểm tra lại hoặc tạo key mới.",
    );
    expect(viTerm("The Aixy server address must use https://, or http:// to a private network.")).toBe(
      "Địa chỉ máy chủ Aixy phải dùng https://, hoặc http:// tới mạng nội bộ.",
    );
    expect(viTerm("MiniMax did not return usage (error 1004).")).toBe("MiniMax không trả về mức dùng (lỗi 1004).");
    expect(viTerm("Could not read the Chutes subscription usage and balance this time.")).toBe(
      "Lần này chưa đọc được mức dùng gói và số dư của Chutes.",
    );
    expect(viTerm("The Helmcode dashboard session expired. Sign in again and paste a fresh Cookie header.")).toBe(
      "Phiên trang quản lý Helmcode đã hết hạn. Hãy đăng nhập lại rồi dán Cookie header mới.",
    );
    expect(viTerm("Paste the whole Cookie header: name=value pairs separated by semicolons")).toBe(
      "Dán nguyên Cookie header: các cặp name=value cách nhau bằng dấu chấm phẩy",
    );
  });

  it("leaves model names and text it does not know in English", () => {
    expect(viTerm("Gemini 3.1 Pro (High)")).toBeUndefined();
    expect(viTerm("GitHub refused Copilot usage for this login.")).toBeUndefined();
  });
});
