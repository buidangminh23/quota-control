import Darwin
import Foundation

/// Keeps the desktop widget on the code of the app that is running (Rules.md §0.61).
///
/// WidgetKit asks a long-lived extension process for timelines. An in-app update moves the old
/// bundle to a temporary folder and deletes it, but the old extension process keeps running from
/// there, so the widget would never run the new code and a widget resized to a new family stays a
/// grey placeholder. At launch the app therefore stops every widget extension process of this user
/// that does not run the bundle's own extension binary, registers that extension again and has the
/// system reload the widgets; the system then starts the current extension on demand.
enum WidgetExtension {
    /// The extension's executable name, which is also its process name.
    static let executableName = "QuotaControlWidget"

    /// One running widget extension process.
    struct Running: Equatable {
        let pid: pid_t
        /// Its executable's path, or `nil` when the system can no longer tell (a deleted file).
        let path: String?
        /// When it started.
        let start: Date
    }

    /// What one pass found and did.
    struct Report {
        let current: URL
        let running: [Running]
        let stale: [Running]
        let stopped: [pid_t]
    }

    /// The extension binary inside the running app's bundle, or `nil` where it has none (a
    /// development build run outside a bundle).
    static func bundledBinary(of bundle: Bundle = .main) -> URL? {
        guard let plugins = bundle.builtInPlugInsURL else { return nil }
        let binary = plugins
            .appendingPathComponent("\(executableName).appex")
            .appendingPathComponent("Contents/MacOS")
            .appendingPathComponent(executableName)
        return FileManager.default.isExecutableFile(atPath: binary.path) ? binary : nil
    }

    /// A process is stale when it does not run the installed binary (its path differs once both
    /// are standardized and their links resolved, or the system no longer knows it), or when it
    /// started before that binary last changed, which an update replacing the file in place leaves
    /// behind.
    static func isStale(path: String?, start: Date, current: String, installed: Date) -> Bool {
        guard let path else { return true }
        if canonical(path) != canonical(current) { return true }
        return start < installed
    }

    /// Every widget extension process the current user runs.
    static func running() -> [Running] {
        let user = getuid()
        return allProcessIDs().compactMap { pid in
            guard pid > 0, let info = bsdInfo(pid), info.pbi_uid == user else { return nil }
            let path = executablePath(pid)
            let name = path.map { URL(fileURLWithPath: $0).lastPathComponent } ?? processName(info)
            guard name == executableName else { return nil }
            let seconds = TimeInterval(info.pbi_start_tvsec) + TimeInterval(info.pbi_start_tvusec) / 1_000_000
            return Running(pid: pid, path: path, start: Date(timeIntervalSince1970: seconds))
        }
    }

    /// Find the stale extension processes and, unless `dryRun`, stop them with `SIGTERM` and
    /// register the bundled extension again. Returns `nil` when the app has no bundled extension.
    static func adopt(bundle: Bundle = .main, dryRun: Bool) -> Report? {
        guard let current = bundledBinary(of: bundle) else { return nil }
        let installed = modificationDate(current) ?? .distantPast
        let running = running()
        let stale = running.filter { isStale(path: $0.path, start: $0.start, current: current.path, installed: installed) }
        guard !dryRun else { return Report(current: current, running: running, stale: stale, stopped: []) }
        let stopped = stale.map(\.pid).filter { kill($0, SIGTERM) == 0 }
        if !stopped.isEmpty {
            register(extension: current.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent())
        }
        return Report(current: current, running: running, stale: stale, stopped: stopped)
    }

    /// Run `adopt` off the main thread, log one line, then call `reload` so the system reloads the
    /// widget timelines from the current extension.
    static func adoptInBackground(reload: @escaping @Sendable () -> Void) {
        DispatchQueue.global(qos: .utility).async {
            if let report = adopt(dryRun: false) {
                NSLog(
                    "Quota Control widget: %d extension process(es), %d stale, %d stopped",
                    report.running.count, report.stale.count, report.stopped.count
                )
            }
            reload()
        }
    }

    /// `path` with its links resolved through the deepest folder that still exists, so a deleted
    /// file and `/var` against `/private/var` compare by where they really are.
    private static func canonical(_ path: String) -> String {
        var existing = URL(fileURLWithPath: path).standardizedFileURL
        var missing: [String] = []
        while existing.path != "/" {
            if let resolved = realpath(existing.path, nil) {
                defer { free(resolved) }
                return missing.reversed().reduce(URL(fileURLWithPath: String(cString: resolved))) {
                    $0.appendingPathComponent($1)
                }.path
            }
            missing.append(existing.lastPathComponent)
            existing.deleteLastPathComponent()
        }
        return URL(fileURLWithPath: path).standardizedFileURL.path
    }

    /// When the binary last changed: the later of its content time and its status time, since an
    /// archive extracted into place keeps the build's content time but gets a new status time.
    private static func modificationDate(_ url: URL) -> Date? {
        var status = stat()
        guard stat(url.path, &status) == 0 else { return nil }
        let seconds = { (time: timespec) in TimeInterval(time.tv_sec) + TimeInterval(time.tv_nsec) / 1_000_000_000 }
        return Date(timeIntervalSince1970: max(seconds(status.st_mtimespec), seconds(status.st_ctimespec)))
    }

    private static func allProcessIDs() -> [pid_t] {
        let estimate = proc_listallpids(nil, 0)
        guard estimate > 0 else { return [] }
        var pids = [pid_t](repeating: 0, count: Int(estimate) + 64)
        let count = pids.withUnsafeMutableBytes { buffer in
            proc_listallpids(buffer.baseAddress, Int32(buffer.count))
        }
        guard count > 0 else { return [] }
        return Array(pids.prefix(Int(count)))
    }

    private static func bsdInfo(_ pid: pid_t) -> proc_bsdinfo? {
        var info = proc_bsdinfo()
        let size = Int32(MemoryLayout<proc_bsdinfo>.stride)
        return proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size ? info : nil
    }

    private static func executablePath(_ pid: pid_t) -> String? {
        var buffer = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
        let length = proc_pidpath(pid, &buffer, UInt32(buffer.count))
        guard length > 0 else { return nil }
        return String(cString: buffer)
    }

    private static func processName(_ info: proc_bsdinfo) -> String {
        var name = info.pbi_name
        return withUnsafeBytes(of: &name) { raw in
            String(decoding: raw.prefix { $0 != 0 }, as: UTF8.self)
        }
    }

    /// Register the extension at `appex` with the plug-in registry; a failure changes nothing,
    /// since the system also finds the extension in the app's bundle on its own.
    private static func register(extension appex: URL) {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/pluginkit")
        task.arguments = ["-a", appex.path]
        task.standardOutput = FileHandle.nullDevice
        task.standardError = FileHandle.nullDevice
        do {
            try task.run()
            task.waitUntilExit()
        } catch {
            NSLog("Quota Control widget: pluginkit could not run: %@", String(describing: error))
        }
    }
}
