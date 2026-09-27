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
  });

  it("leaves model names and text it does not know in English", () => {
    expect(viTerm("Gemini 3.1 Pro (High)")).toBeUndefined();
    expect(viTerm("GitHub refused Copilot usage for this login.")).toBeUndefined();
  });
});
