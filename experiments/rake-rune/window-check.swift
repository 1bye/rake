// macOS window captures and measurements. Build with swiftc -O window-check.swift.
import AppKit
import Darwin
import ImageIO
import UniformTypeIdentifiers

struct Failure: Error, CustomStringConvertible {
    var description: String
    init(_ message: String) { description = message }
}

func now() -> Double { ProcessInfo.processInfo.systemUptime }
func epochMs() -> Double { Date().timeIntervalSince1970 * 1000 }

struct Window {
    let id: CGWindowID
    let bounds: CGRect
    let time: Double

    func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
        // A 640-point client area sits under the native title bar.
        let edge = (bounds.width - 960) / 2
        return CGPoint(x: bounds.minX + edge + x, y: bounds.maxY - edge - 640 + y)
    }
}

func visibleWindow(_ pid: Int32) -> Window? {
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
    for info in (windows as? [[String: Any]]) ?? [] {
        guard info[kCGWindowOwnerPID as String] as? Int32 == pid,
              info[kCGWindowLayer as String] as? Int == 0,
              let id = info[kCGWindowNumber as String] as? CGWindowID,
              let geometry = info[kCGWindowBounds as String] as? [String: CGFloat],
              let bounds = CGRect(dictionaryRepresentation: geometry as CFDictionary),
              bounds.width >= 900 && bounds.height >= 600
        else { continue }

        return Window(id: id, bounds: bounds, time: now())
    }
    return nil
}

func waitForWindow(_ process: Process) throws -> Window {
    let deadline = now() + 25
    while now() < deadline && process.isRunning {
        if let window = visibleWindow(process.processIdentifier) { return window }
        Thread.sleep(forTimeInterval: 0.002)
    }

    throw Failure("main window never appeared for PID \(process.processIdentifier)")
}

func usage(_ pid: Int32) throws -> rusage_info_v0 {
    var result = rusage_info_v0()
    let status = withUnsafeMutablePointer(to: &result) { data in
        data.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) {
            proc_pid_rusage(pid, RUSAGE_INFO_V0, $0)
        }
    }
    guard status == 0 else { throw Failure("proc_pid_rusage failed with errno \(errno)") }
    return result
}

func nanosecondsPerTick() throws -> Double {
    var timebase = mach_timebase_info_data_t()
    guard mach_timebase_info(&timebase) == KERN_SUCCESS, timebase.denom != 0 else {
        throw Failure("could not read the Mach timebase")
    }
    return Double(timebase.numer) / Double(timebase.denom)
}

func cpuTicks(_ before: rusage_info_v0, _ after: rusage_info_v0) -> UInt64 {
    (after.ri_user_time - before.ri_user_time) + (after.ri_system_time - before.ri_system_time)
}

func requireFrontmost(_ pid: Int32) throws {
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
        throw Failure("the evaluation window lost focus during the input run")
    }
}

func focus(_ pid: Int32) throws {
    _ = NSRunningApplication(processIdentifier: pid)?.activate(options: [])
    let deadline = now() + 4
    while now() < deadline {
        if NSWorkspace.shared.frontmostApplication?.processIdentifier == pid { return }
        Thread.sleep(forTimeInterval: 0.025)
    }
    try requireFrontmost(pid)
}

func mouse(_ kind: CGEventType, _ p: CGPoint) throws {
    guard let event = CGEvent(mouseEventSource: nil, mouseType: kind, mouseCursorPosition: p, mouseButton: .left)
    else { throw Failure("could not create mouse event") }
    if kind == .leftMouseDown || kind == .leftMouseUp {
        event.setIntegerValueField(.mouseEventClickState, value: 1)
    }
    event.post(tap: .cghidEventTap)
}

func click(_ window: Window, _ pid: Int32, _ x: CGFloat, _ y: CGFloat) throws {
    try requireFrontmost(pid)
    let p = window.point(x, y)
    try mouse(.mouseMoved, p)
    try mouse(.leftMouseDown, p)
    try mouse(.leftMouseUp, p)
}

func scroll(_ window: Window, _ pid: Int32) throws {
    let point = window.point(420, 300)
    try requireFrontmost(pid)
    try mouse(.mouseMoved, point)
    for _ in 0..<80 {
        try requireFrontmost(pid)
        guard let event = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: -34, wheel2: 0, wheel3: 0)
        else { throw Failure("could not create scroll event") }
        event.location = point
        event.post(tap: .cghidEventTap)
        Thread.sleep(forTimeInterval: 0.016)
    }
}

func capture(_ window: Window, _ output: URL) throws {
    let raw = output.deletingLastPathComponent().appendingPathComponent("window-\(window.id).png")
    defer { try? FileManager.default.removeItem(at: raw) }

    let command = Process()
    command.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    command.arguments = ["-x", "-o", "-t", "png", "-l\(window.id)", raw.path]
    try command.run()
    command.waitUntilExit()

    guard command.terminationStatus == 0,
          let source = CGImageSourceCreateWithURL(raw as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
    else { throw Failure("could not capture window \(window.id) at \(window.bounds)") }

    guard (1920...1924).contains(image.width), image.height >= 1280 else {
        throw Failure("unexpected capture dimensions \(image.width) × \(image.height) at \(window.bounds)")
    }

    let edge = (image.width - 1920) / 2
    guard let crop = image.cropping(to: CGRect(x: edge, y: image.height - edge - 1280, width: 1920, height: 1280)),
          let destination = CGImageDestinationCreateWithURL(output as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { throw Failure("could not crop \(output.path)") }

    CGImageDestinationAddImage(destination, crop, nil)
    guard CGImageDestinationFinalize(destination) else { throw Failure("could not save \(output.path)") }
}

func withApp(_ binary: URL, _ log: URL, arguments: [String] = [], profile: Bool = false,
             action: (Window, Int32, Double) throws -> Void) throws {
    try Data().write(to: log)
    let output = try FileHandle(forWritingTo: log)
    defer { try? output.close() }

    let app = Process()
    app.executableURL = binary
    app.arguments = arguments
    app.standardOutput = output
    app.standardError = output
    var env = ProcessInfo.processInfo.environment
    env["RAKE_EVAL_FRAMES"] = profile ? "1" : nil
    app.environment = env

    let start = now()
    try app.run()
    defer {
        if app.isRunning { app.terminate() }
        app.waitUntilExit()
    }

    let window = try waitForWindow(app)
    try action(window, app.processIdentifier, start)
}

func run() throws {
    let args = Array(CommandLine.arguments.dropFirst())
    guard args.count == 4, ["capture", "startup-idle", "frames", "workload"].contains(args[0]),
          ["rune", "gpui"].contains(args[1]) else {
        throw Failure("usage: window-check capture|startup-idle|frames|workload rune|gpui /path/to/app /path/to/output-dir")
    }

    let mode = args[0]
    let stack = args[1]
    let binary = URL(fileURLWithPath: args[2])
    let output = URL(fileURLWithPath: args[3], isDirectory: true)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)

    if mode == "startup-idle" {
        let nanosPerTick = try nanosecondsPerTick()
        for run in 1...5 {
            try withApp(binary, output.appendingPathComponent("\(stack)-idle-\(run).log")) { window, pid, start in
                Thread.sleep(forTimeInterval: 2)
                let before = try usage(pid)
                let idleStart = now()
                Thread.sleep(forTimeInterval: 5)
                let after = try usage(pid)
                let elapsed = now() - idleStart
                let cpuPercent = Double(cpuTicks(before, after)) * nanosPerTick / 1e7 / elapsed
                print("startup-idle,\(stack),\(run),\((window.time - start) * 1000),\(elapsed),\(cpuPercent),\(after.ri_interrupt_wkups - before.ri_interrupt_wkups),\(after.ri_phys_footprint)")
            }
        }
        return
    }

    if mode == "capture" {
        guard CGPreflightScreenCaptureAccess() else { throw Failure("macOS Screen Recording permission is required") }
        for route in ["overview", "activity", "settings"] {
            let arguments = route == "overview" ? [] : stack == "rune" ? ["--scene", route] : ["/" + route]
            try withApp(binary, output.appendingPathComponent("\(stack)-\(route).log"), arguments: arguments) { window, pid, _ in
                Thread.sleep(forTimeInterval: 1)
                try capture(visibleWindow(pid) ?? window, output.appendingPathComponent("\(stack)-\(route).png"))
            }
        }
        return
    }

    guard CGPreflightPostEventAccess() else { throw Failure("macOS Accessibility permission is required to post input events") }

    let nanosPerTick = try nanosecondsPerTick()
    for run in 1...3 {
        try withApp(binary, output.appendingPathComponent("\(stack)-\(mode)-\(run).log"), profile: mode == "frames") { initial, pid, _ in
            Thread.sleep(forTimeInterval: 1)
            try focus(pid)
            let cursor = CGEvent(source: nil)?.location
            defer { if let cursor { CGWarpMouseCursorPosition(cursor) } }
            let window = visibleWindow(pid) ?? initial
            let motionY: CGFloat = stack == "rune" ? 255 : 282
            func measure(_ scenario: String, _ action: () throws -> Void) throws {
                let before = mode == "workload" ? try usage(pid) : nil
                let start = epochMs()
                try action()
                Thread.sleep(forTimeInterval: 0.5)
                try requireFrontmost(pid)
                let end = epochMs()
                if let before {
                    let cpuMs = Double(cpuTicks(before, try usage(pid))) * nanosPerTick / 1e6
                    print("workload,\(stack),\(run),\(scenario),\(start),\(end),\(pid),\(cpuMs)")
                } else {
                    print("frames,\(stack),\(run),\(scenario),\(start),\(end)")
                }
            }
            try measure("animation") {
                for _ in 0..<12 {
                    try click(window, pid, 266, motionY)
                    Thread.sleep(forTimeInterval: 0.100)
                }
            }
            try click(window, pid, 80, 122)
            Thread.sleep(forTimeInterval: 0.5)
            try measure("scroll") { try scroll(window, pid) }
        }
    }
}

do {
    try run()
} catch {
    FileHandle.standardError.write(Data("window-check: \(error)\n".utf8))
    exit(1)
}
