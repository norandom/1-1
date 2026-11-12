// Task 2.4 spike — OS-level window probe (no WebDriver involved): exits 0
// when the given PID owns an on-screen window at least 400x300 px (the
// Drift window is 1200x800; tao also creates tiny helper windows — the 2.3
// Windows spike documented that trap, so small windows are ignored here).
//
// Uses CGWindowListCopyWindowInfo: window bounds and owner PID are not
// TCC-gated (unlike window titles), so this needs no Screen Recording /
// Accessibility permission on the runner.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
if args.count < 2 || Int32(args[1]) == nil {
    FileHandle.standardError.write("usage: window-probe.swift <pid>\n".data(using: .utf8)!)
    exit(2)
}
let pid = Int32(args[1])!

let options: CGWindowListOption = [.optionOnScreenOnly]
guard let windowList = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
    FileHandle.standardError.write("CGWindowListCopyWindowInfo failed\n".data(using: .utf8)!)
    exit(3)
}

for window in windowList {
    if let ownerPID = window[kCGWindowOwnerPID as String] as? Int32,
       ownerPID == pid,
       let bounds = window[kCGWindowBounds as String] as? [String: Any],
       let width = (bounds["Width"] as? NSNumber)?.doubleValue,
       let height = (bounds["Height"] as? NSNumber)?.doubleValue,
       width >= 400, height >= 300 {
        print("drift window on-screen: \(Int(width))x\(Int(height)) for pid \(pid)")
        exit(0)
    }
}

FileHandle.standardError.write("no on-screen window >= 400x300 for pid \(pid)\n".data(using: .utf8)!)
exit(1)
