import 'package:flutter/widgets.dart';

/// The one width that changes the layout.
///
/// Deliberately a single threshold. The only structural question this app has
/// is "is there room for a sidebar and a note side by side"; every extra
/// breakpoint would multiply the states that need testing for a distinction
/// nothing actually makes. Tablet portrait stays on the phone layout, which is
/// the honest answer at that width.
const kExpandedWidth = 900.0;

extension Layout on BuildContext {
  /// Wide enough for two panes.
  ///
  /// `MediaQuery.sizeOf` rather than a `LayoutBuilder`: this is a property of
  /// the *window*, so every widget has to agree on it regardless of the box it
  /// happens to be laid out in — and it rebuilds on resize, which a browser
  /// does continuously while the user drags an edge.
  bool get isExpanded => MediaQuery.sizeOf(this).width >= kExpandedWidth;
}
