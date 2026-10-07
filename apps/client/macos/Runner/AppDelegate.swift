import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return true
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
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
