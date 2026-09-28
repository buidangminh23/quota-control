import { useEffect, useRef, useState } from "react";
import { backend } from "@/lib/backend";
import { useSettings } from "@/state/hooks";
import { useApp } from "@/state/store";
import { Button } from "../ui/controls";
import { Section } from "./parts";
import "./BugReport.css";

const translations = {
  vi: {
    heading: "Báo cáo lỗi", open: "Viết báo cáo lỗi", title: "Tiêu đề lỗi", description: "Lỗi gặp phải và kết quả mong đợi", steps: "Các bước tái hiện",
    privacy: "Báo cáo sẽ công khai trên GitHub và cần tài khoản GitHub để gửi. Không điền mật khẩu, API key hoặc thông tin cá nhân. App chỉ đính kèm phiên bản, hệ điều hành và ngôn ngữ; không thu thập nhật ký hay tài khoản.",
    preview: "Xem nội dung gửi", copy: "Sao chép báo cáo", submit: "Mở bản nháp trên GitHub", copied: "Đã sao chép báo cáo.", opened: "Đã mở bản nháp. Kiểm tra nội dung và bấm gửi trên GitHub để hoàn tất.",
    long: "Báo cáo dài: hãy sao chép nội dung rồi dán vào bản nháp trên GitHub.", failed: "Không thực hiện được. Hãy thử lại hoặc sao chép báo cáo và mở GitHub thủ công.", link: "Mở trang Issues", cancel: "Đóng biểu mẫu",
  },
  en: {
    heading: "Bug reports", open: "Report a bug", title: "Bug title", description: "What happened and what you expected", steps: "Steps to reproduce",
    privacy: "Reports are public on GitHub and require a GitHub account to submit. Do not include passwords, API keys or personal information. The app attaches only its version, operating system and language; no logs or accounts are collected.",
    preview: "Preview report", copy: "Copy report", submit: "Open draft on GitHub", copied: "Report copied.", opened: "Draft opened. Review it and submit on GitHub to finish.",
    long: "Long report: copy the content and paste it into the GitHub draft.", failed: "Could not complete the action. Retry or copy the report and open GitHub manually.", link: "Open Issues", cancel: "Close form",
  },
};

export function BugReport() {
  const { language } = useSettings();
  const info = useApp((state) => state.info);
  const text = translations[language];
  const [expanded, setExpanded] = useState(false);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [steps, setSteps] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const requested = useApp((state) => state.bugReportRequested);
  const titleRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!requested) return;
    setExpanded(true);
  }, [requested]);
  useEffect(() => {
    if (!requested || !expanded) return;
    titleRef.current?.scrollIntoView?.({ block: "center" });
    titleRef.current?.focus({ preventScroll: true });
    useApp.setState({ bugReportRequested: false });
  }, [requested, expanded]);
  const body = `## Description / Expected behavior\n${description.trim()}\n\n## Steps to reproduce\n${steps.trim()}\n\n## App information\n- Version: ${info?.version ?? "unknown"}\n- Platform: ${info?.platform ?? "unknown"}\n- Language: ${language}`;
  const url = new URL("https://github.com/buidangminh23/quota-control/issues/new");
  url.searchParams.set("title", title.trim());
  url.searchParams.set("body", body);
  const long = url.href.length > 7500;
  if (long) url.searchParams.delete("body");
  const run = async (action: () => Promise<void>, success: string) => {
    setBusy(true);
    setStatus("");
    try { await action(); setStatus(success); }
    catch { setStatus(text.failed); }
    finally { setBusy(false); }
  };
  return (
    <Section title={text.heading}>
      <div className="uc-bug-report">
        <Button onClick={() => setExpanded(!expanded)}>{expanded ? text.cancel : text.open}</Button>
        {expanded ? <>
          <p>{text.privacy}</p>
          <label>{text.title}<input ref={titleRef} maxLength={120} value={title} onChange={(event) => setTitle(event.target.value)} /></label>
          <label>{text.description}<textarea rows={4} maxLength={4000} value={description} onChange={(event) => setDescription(event.target.value)} /></label>
          <label>{text.steps}<textarea rows={3} maxLength={4000} value={steps} onChange={(event) => setSteps(event.target.value)} /></label>
          <details><summary>{text.preview}</summary><pre>{body}</pre></details>
          {long ? <p>{text.long}</p> : null}
          <Button disabled={busy || !description.trim()} onClick={() => void run(() => backend().copyText(`${title.trim()}\n\n${body}`), text.copied)}>{text.copy}</Button>
          <Button disabled={busy || !title.trim() || !description.trim()} onClick={() => void run(() => backend().openUrl(url.href), long ? text.long : text.opened)}>{text.submit}</Button>
          <a href="https://github.com/buidangminh23/quota-control/issues" onClick={(event) => { event.preventDefault(); void run(() => backend().openUrl("https://github.com/buidangminh23/quota-control/issues"), ""); }}>{text.link}</a>
          <p role="status">{status}</p>
        </> : null}
      </div>
    </Section>
  );
}
