cask "quota-control" do
  version "0.2.0"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/buidangminh23/quota-control/releases/download/v#{version}/Quota-Control_#{version}_aarch64.dmg"
  name "Quota Control"
  desc "Menu bar limits for Claude, Codex, Cursor and other AI coding tools"
  homepage "https://github.com/buidangminh23/quota-control"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates true
  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Quota Control.app"
  binary "#{appdir}/Quota Control.app/Contents/MacOS/usagectl"

  postflight_steps do
    run "/usr/bin/xattr",
        args:         ["-dr", "com.apple.quarantine", "{{appdir}}/Quota Control.app"],
        must_succeed: false
    run "/usr/bin/pluginkit",
        args:         ["-a", "{{appdir}}/Quota Control.app/Contents/PlugIns/QuotaControlWidget.appex"],
        must_succeed: false
  end

  uninstall_postflight_steps do
    unless_path_exists "{{appdir}}/Quota Control.app" do
      remove "~/.local/bin/usagectl", symlink_target_contains: "Quota Control.app/Contents/MacOS/"
    end
  end

  uninstall signal: ["TERM", "com.buidangminh.usagecontrol"]

  zap trash: [
    "~/Library/Application Scripts/com.buidangminh.usagecontrol.widget",
    "~/Library/Application Support/usage-control",
    "~/Library/Caches/com.buidangminh.usagecontrol",
    "~/Library/Caches/usage-control",
    "~/Library/Containers/com.buidangminh.usagecontrol.widget",
    "~/Library/HTTPStorages/com.buidangminh.usagecontrol",
    "~/Library/LaunchAgents/Usage Control.plist",
    "~/Library/Logs/usage-control",
    "~/Library/Preferences/com.buidangminh.usagecontrol.plist",
    "~/Library/Saved Application State/com.buidangminh.usagecontrol.savedState",
    "~/Library/WebKit/com.buidangminh.usagecontrol",
  ]
end
