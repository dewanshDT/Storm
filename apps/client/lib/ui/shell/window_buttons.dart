import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:window_manager/window_manager.dart';

import '../tokens.dart';

/// Who draws the window's buttons (decision 153d).
enum WindowChrome {
  /// The OS's traffic lights, over the bar; Storm leaves room for them.
  mac,

  /// Storm's own minimise, maximise and close.
  caption,

  /// Web, phones and tablets: the browser or the OS owns the window.
  none,
}

WindowChrome get windowChrome {
  if (kIsWeb) return WindowChrome.none;
  return switch (defaultTargetPlatform) {
    TargetPlatform.macOS => WindowChrome.mac,
    TargetPlatform.windows || TargetPlatform.linux => WindowChrome.caption,
    _ => WindowChrome.none,
  };
}

/// Before the first frame. macOS hides its title bar natively and Linux drops
/// its header bar natively; Windows asks window_manager to.
Future<void> initWindowChrome() async {
  if (windowChrome == WindowChrome.none) return;
  await windowManager.ensureInitialized();
  if (defaultTargetPlatform == TargetPlatform.windows) {
    await windowManager.setTitleBarStyle(
      TitleBarStyle.hidden,
      windowButtonVisibility: false,
    );
  }
}

/// A window call that is a no-op where no plugin answers (tests).
Future<T?> _window<T>(Future<T> Function() call) async {
  try {
    return await call();
  } on MissingPluginException {
    return null;
  }
}

@immutable
class WindowState {
  const WindowState({
    this.focused = true,
    this.fullScreen = false,
    this.maximized = false,
  });

  final bool focused;
  final bool fullScreen;
  final bool maximized;

  WindowState copyWith({bool? focused, bool? fullScreen, bool? maximized}) =>
      WindowState(
        focused: focused ?? this.focused,
        fullScreen: fullScreen ?? this.fullScreen,
        maximized: maximized ?? this.maximized,
      );
}

class WindowStateNotifier extends Notifier<WindowState> with WindowListener {
  @override
  WindowState build() {
    if (windowChrome == WindowChrome.none) return const WindowState();
    windowManager.addListener(this);
    ref.onDispose(() => windowManager.removeListener(this));
    unawaited(_read());
    return const WindowState();
  }

  Future<void> _read() async {
    final focused = await _window(windowManager.isFocused);
    final fullScreen = await _window(windowManager.isFullScreen);
    final maximized = await _window(windowManager.isMaximized);
    state = state.copyWith(
      focused: focused,
      fullScreen: fullScreen,
      maximized: maximized,
    );
  }

  @override
  void onWindowFocus() => state = state.copyWith(focused: true);
  @override
  void onWindowBlur() => state = state.copyWith(focused: false);
  @override
  void onWindowMaximize() => state = state.copyWith(maximized: true);
  @override
  void onWindowUnmaximize() => state = state.copyWith(maximized: false);
  @override
  void onWindowEnterFullScreen() => state = state.copyWith(fullScreen: true);
  @override
  void onWindowLeaveFullScreen() => state = state.copyWith(fullScreen: false);

  void minimize() => unawaited(_window(windowManager.minimize));
  void close() => unawaited(_window(windowManager.close));
  void drag() => unawaited(_window(windowManager.startDragging));

  /// Maximise on Windows and Linux, zoom on macOS: the bar's double-click.
  void toggleMaximized() => unawaited(
    _window(() async {
      if (await windowManager.isMaximized()) {
        await windowManager.unmaximize();
      } else {
        await windowManager.maximize();
      }
    }),
  );
}

final windowStateProvider = NotifierProvider<WindowStateNotifier, WindowState>(
  WindowStateNotifier.new,
);

enum CaptionButton { minimize, maximize, close }

/// Which caption buttons sit at each end of the bar.
typedef CaptionLayout = ({List<CaptionButton> left, List<CaptionButton> right});

const CaptionLayout kDefaultCaptionLayout = (
  left: [],
  right: [CaptionButton.minimize, CaptionButton.maximize, CaptionButton.close],
);

/// Reads GTK's `gtk-decoration-layout`, e.g. `close,minimize:` or
/// `appmenu:minimize,maximize,close`. Names Storm doesn't draw are skipped.
CaptionLayout parseDecorationLayout(String layout) {
  List<CaptionButton> side(String s) => [
    for (final name in s.split(','))
      ?switch (name.trim()) {
        'minimize' => CaptionButton.minimize,
        'maximize' => CaptionButton.maximize,
        'close' => CaptionButton.close,
        _ => null,
      },
  ];
  final cut = layout.indexOf(':');
  if (cut < 0) return (left: side(layout), right: const []);
  return (
    left: side(layout.substring(0, cut)),
    right: side(layout.substring(cut + 1)),
  );
}

const _windowChannel = MethodChannel('storm/window');

/// The desktop's button order on Linux (the runner answers on
/// `storm/window`); Windows' own everywhere else.
final captionLayoutProvider = FutureProvider<CaptionLayout>((ref) async {
  if (kIsWeb || defaultTargetPlatform != TargetPlatform.linux) {
    return kDefaultCaptionLayout;
  }
  try {
    final layout = await _windowChannel.invokeMethod<String>(
      'decorationLayout',
    );
    if (layout == null || layout.isEmpty) return kDefaultCaptionLayout;
    return parseDecorationLayout(layout);
  } on MissingPluginException {
    return kDefaultCaptionLayout;
  } on PlatformException {
    return kDefaultCaptionLayout;
  }
});

/// One end's caption buttons: 46 wide, the full bar height.
class CaptionButtons extends ConsumerWidget {
  const CaptionButtons({super.key, required this.buttons});

  final List<CaptionButton> buttons;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final window = ref.watch(windowStateProvider);
    final controls = ref.read(windowStateProvider.notifier);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final b in buttons)
          switch (b) {
            CaptionButton.minimize => _Caption(
              key: const Key('caption-minimize'),
              icon: LucideIcons.minus,
              label: 'Minimise',
              focused: window.focused,
              onTap: controls.minimize,
            ),
            CaptionButton.maximize => _Caption(
              key: const Key('caption-maximize'),
              icon: window.maximized ? LucideIcons.copy : LucideIcons.square,
              label: window.maximized ? 'Restore' : 'Maximise',
              focused: window.focused,
              onTap: controls.toggleMaximized,
            ),
            CaptionButton.close => _Caption(
              key: const Key('caption-close'),
              icon: LucideIcons.x,
              label: 'Close',
              focused: window.focused,
              danger: true,
              onTap: controls.close,
            ),
          },
      ],
    );
  }
}

double captionWidth(StormTokens t) => t.sp * 5.75;

class _Caption extends StatefulWidget {
  const _Caption({
    super.key,
    required this.icon,
    required this.label,
    required this.focused,
    required this.onTap,
    this.danger = false,
  });

  final IconData icon;
  final String label;
  final bool focused;
  final VoidCallback onTap;
  final bool danger;

  @override
  State<_Caption> createState() => _CaptionState();
}

class _CaptionState extends State<_Caption> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final fill = !_hover
        ? Colors.transparent
        : (widget.danger ? t.danger : t.surface2);
    final ink = _hover
        ? (widget.danger ? t.onAccent : t.text)
        : (widget.focused ? t.text2 : t.text3);
    return Semantics(
      button: true,
      label: widget.label,
      excludeSemantics: true,
      onTap: widget.onTap,
      child: MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          onTap: widget.onTap,
          child: Container(
            width: captionWidth(t),
            color: fill,
            alignment: Alignment.center,
            child: Icon(widget.icon, size: t.sp * 1.75, color: ink),
          ),
        ),
      ),
    );
  }
}
