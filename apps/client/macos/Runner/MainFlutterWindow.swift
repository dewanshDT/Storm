import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  private var frameObservers: [NSObjectProtocol] = []

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    // Storm draws its own 44px app bar under the traffic lights (decision
    // 153d): no OS title, and the content runs to the window's top edge.
    self.titleVisibility = .hidden
    self.titlebarAppearsTransparent = true
    self.styleMask.insert(.fullSizeContentView)

    RegisterGeneratedPlugins(registry: flutterViewController)
    StormLinks.shared.attach(flutterViewController.engine.binaryMessenger)
    StormClipboard.attach(flutterViewController.engine.binaryMessenger)

    super.awakeFromNib()

    // AppKit lays the title bar out again on these, putting the buttons
    // back where it keeps them.
    for name in [
      NSWindow.didResizeNotification,
      NSWindow.didEndLiveResizeNotification,
      NSWindow.didExitFullScreenNotification,
      NSWindow.didBecomeKeyNotification,
      NSWindow.didResignKeyNotification,
    ] {
      NotificationCenter.default.addObserver(
        forName: name, object: self, queue: .main
      ) { [weak self] _ in self?.placeTrafficLights() }
    }
    // AppKit also moves them back on any title bar layout, focused or not,
    // so follow the views themselves.
    for view in [
      standardWindowButton(.closeButton),
      standardWindowButton(.closeButton)?.superview?.superview,
    ].compactMap({ $0 }) {
      view.postsFrameChangedNotifications = true
      frameObservers.append(
        NotificationCenter.default.addObserver(
          forName: NSView.frameDidChangeNotification, object: view, queue: .main
        ) { [weak self] _ in self?.placeTrafficLights() })
    }
    placeTrafficLights()
    DispatchQueue.main.async { [weak self] in self?.placeTrafficLights() }
  }

  /// The traffic lights at x 20, centred 2px below the middle of the bar,
  /// which is where `StormAppBar` leaves room for them.
  private func placeTrafficLights() {
    guard !styleMask.contains(.fullScreen),
      let close = standardWindowButton(.closeButton),
      let mini = standardWindowButton(.miniaturizeButton),
      let zoom = standardWindowButton(.zoomButton),
      let titlebar = close.superview?.superview
    else { return }
    let barHeight: CGFloat = 44
    var frame = titlebar.frame
    frame.size.height = barHeight
    frame.origin.y = self.frame.height - barHeight
    if titlebar.frame != frame { titlebar.frame = frame }

    let spacing = mini.frame.minX - close.frame.minX
    let centreFromBottom = barHeight / 2 - 1
    for (i, button) in [close, mini, zoom].enumerated() {
      let origin = NSPoint(
        x: 20 + CGFloat(i) * spacing,
        y: centreFromBottom - button.frame.height / 2)
      // Setting an unchanged origin would post another frame change.
      if button.frame.origin != origin { button.setFrameOrigin(origin) }
    }
  }
}
