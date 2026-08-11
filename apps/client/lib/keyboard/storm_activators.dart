import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// Whether Storm treats this platform as Apple-modifier (⌘) rather than Ctrl.
///
/// Uses [defaultTargetPlatform] so Mac web gets Meta with the rest of macOS,
/// and so widget tests can flip it with [debugDefaultTargetPlatformOverride].
bool get stormUsesMetaModifier => defaultTargetPlatform == TargetPlatform.macOS;

/// Whether this platform has a physical keyboard worth arming chords for.
///
/// Desktop and desktop-class web only. On a phone the global Focus would grab
/// primary focus to arm nothing, and could fight a field that autofocuses when
/// its screen opens (the search field does).
bool get stormHasKeyboard => switch (defaultTargetPlatform) {
  TargetPlatform.macOS ||
  TargetPlatform.linux ||
  TargetPlatform.windows ||
  TargetPlatform.fuchsia => true,
  _ => false,
};

/// Platform-aware chord: Meta on macOS, Control elsewhere.
///
/// Prefer this over hardcoding [LogicalKeyboardKey.meta] so Windows/Linux and
/// non-Mac browsers get Ctrl without a second map.
SingleActivator stormActivator(LogicalKeyboardKey key, {bool shift = false}) {
  return SingleActivator(
    key,
    meta: stormUsesMetaModifier,
    control: !stormUsesMetaModifier,
    shift: shift,
  );
}
