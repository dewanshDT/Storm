import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  // Decision 161, slice 161-4: off by default, so closing the window
  // quits as it always has unless the person opts in from Settings ›
  // Notifications › This device.
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return !StormLifecycle.keepRunningWhenClosed
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  // With the window closed rather than quit, the Dock icon is the only
  // way back in; without this, clicking it does nothing once every
  // window has gone (AppKit's default reopen only unhides/unminimizes
  // existing windows, and a *closed* one is neither).
  override func applicationShouldHandleReopen(
    _ sender: NSApplication, hasVisibleWindows flag: Bool
  ) -> Bool {
    if !flag {
      for window in sender.windows {
        window.makeKeyAndOrderFront(nil)
      }
    }
    return true
  }

  // Integration sign-ins come back as storm://oauth links (spec §10.4,
  // decision 81l). Info.plist registers the scheme.
  override func application(_ application: NSApplication, open urls: [URL]) {
    for url in urls {
      StormLinks.shared.deliver(url)
    }
  }
}

/// Hands `storm://oauth` redirects to Dart on the `storm/links` channel.
///
/// Buffered until Dart calls `takeLinks`, then forwarded as `link` calls: a
/// redirect that launched the app arrives before Dart is listening, and must
/// not be lost to that race.
final class StormLinks {
  static let shared = StormLinks()

  private var channel: FlutterMethodChannel?
  private var buffered: [String] = []
  private var dartReady = false

  func attach(_ messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "storm/links", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self = self, call.method == "takeLinks" else {
        result(FlutterMethodNotImplemented)
        return
      }
      result(self.buffered)
      self.buffered.removeAll()
      self.dartReady = true
    }
    self.channel = channel
  }

  /// Only `storm://oauth…`: nothing else is routed into the app.
  func deliver(_ url: URL) {
    guard url.scheme == "storm", url.host == "oauth" else { return }
    if dartReady, let channel = channel {
      channel.invokeMethod("link", arguments: url.absoluteString)
    } else {
      buffered.append(url.absoluteString)
    }
  }
}

/// Hands the clipboard's image to Dart on `storm/clipboard` (D15 AM45): an
/// image's bytes as they are, or a screenshot's TIFF as PNG. A copied file
/// that is not an image is no image, even though Finder puts its icon there.
enum StormClipboard {
  static func attach(_ messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "storm/clipboard", binaryMessenger: messenger)
    channel.setMethodCallHandler { call, result in
      guard call.method == "readImage" else {
        result(FlutterMethodNotImplemented)
        return
      }
      result(readImage().map { FlutterStandardTypedData(bytes: $0) })
    }
  }

  static func readImage() -> Data? {
    let board = NSPasteboard.general
    let options: [NSPasteboard.ReadingOptionKey: Any] = [.urlReadingFileURLsOnly: true]
    if let urls = board.readObjects(forClasses: [NSURL.self], options: options) as? [URL],
       let url = urls.first {
      guard NSImage(contentsOf: url) != nil else { return nil }
      let kept = ["png", "jpg", "jpeg", "gif", "webp"]
      if kept.contains(url.pathExtension.lowercased()) {
        return try? Data(contentsOf: url)
      }
      return png(NSImage(contentsOf: url))
    }
    if let data = board.data(forType: .png) { return data }
    if let tiff = board.data(forType: .tiff), let rep = NSBitmapImageRep(data: tiff) {
      return rep.representation(using: .png, properties: [:])
    }
    return nil
  }

  private static func png(_ image: NSImage?) -> Data? {
    guard let tiff = image?.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff) else {
      return nil
    }
    return rep.representation(using: .png, properties: [:])
  }
}

/// Decision 161, slice 161-4: "Keep Storm running when its window is
/// closed". `keepRunningWhenClosed` is read directly by
/// `applicationShouldTerminateAfterLastWindowClosed` — a `UserDefaults`
/// key rather than a round trip through Dart on every window close.
enum StormLifecycle {
  private static let key = "stormKeepRunningWhenClosed"

  static var keepRunningWhenClosed: Bool {
    UserDefaults.standard.bool(forKey: key)
  }

  static func attach(_ messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "storm/lifecycle", binaryMessenger: messenger)
    channel.setMethodCallHandler { call, result in
      guard call.method == "setKeepRunningWhenClosed", let keep = call.arguments as? Bool else {
        result(FlutterMethodNotImplemented)
        return
      }
      UserDefaults.standard.set(keep, forKey: key)
      result(nil)
    }
  }
}
