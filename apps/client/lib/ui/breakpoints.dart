import 'package:flutter/widgets.dart';

/// The one width that changes the layout.
///
/// Deliberately a single threshold. The only structural question this app has
/// is "is there room for a sidebar and a note side by side"; every extra
/// breakpoint would multiply the states that need testing for a distinction
/// nothing actually makes. Tablet portrait stays on the phone layout, which is
/// the honest answer at that width.
const kExpandedWidth = 900.0;

/// The prototype's frame, and the widths it draws inside it.
///
/// The design is a 1200px mock with a 260px sidebar, a 56px rail and a 280px
/// properties drawer. Pinning those three numbers is what made a 1700px window
/// look wrong: the side columns stayed at their 1200px size and the note pane
/// absorbed every extra pixel, so the same layout that matches the mockup
/// exactly at 1200 reads as a narrow sidebar beside an enormous editor at 1700.
/// The ratios are what the design actually specifies; the pixel values are
/// those ratios evaluated at one width.
const _designFrame = 1200.0;
const _designSidebar = 260.0;
const _designDrawer = 280.0;

/// Where the note's prose sits inside its column — `padding: 28px 40px 36px`
/// in the prototype.
const kEditorInset = 40.0;

/// The measure. Long-form text stops growing here however wide the window is.
const kEditorMeasure = 640.0;

extension Layout on BuildContext {
  /// Wide enough for two panes.
  ///
  /// `MediaQuery.sizeOf` rather than a `LayoutBuilder`: this is a property of
  /// the *window*, so every widget has to agree on it regardless of the box it
  /// happens to be laid out in — and it rebuilds on resize, which a browser
  /// does continuously while the user drags an edge.
  bool get isExpanded => MediaQuery.sizeOf(this).width >= kExpandedWidth;

  /// The sidebar's share of the window, never below the design's own 260 and
  /// capped so a 4K display does not hand it a third of the screen.
  double get sidebarWidth => _column(_designSidebar, 400);

  /// The properties drawer's share, on the same rule.
  double get drawerWidth => _column(_designDrawer, 430);

  double _column(double atDesignFrame, double cap) {
    final width = MediaQuery.sizeOf(this).width;
    return (width * atDesignFrame / _designFrame).clamp(atDesignFrame, cap);
  }
}
