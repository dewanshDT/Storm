import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../keyboard/storm_activators.dart';
import '../../state/health.dart';
import '../palette/palette.dart' show openPalette;
import '../surfaces.dart';
import '../tokens.dart';
import 'nav_history.dart';
import 'window_buttons.dart';

/// The app bar's height (Window Chrome B). macOS centres its traffic lights
/// in it natively (`MainFlutterWindow.swift`), so the two must agree.
double appBarHeight(StormTokens t) => t.sp * 5.5;

/// Where the first thing under the bar starts, so the rail's first icon, the
/// sidebar's first row and the page title share one top line (153a).
double railTopInset(StormTokens t) => t.sp;
double sidebarTopInset(StormTokens t) => t.sp * 1.5;
double pageTopInset(StormTokens t) => t.sp * 1.75;

/// macOS's three traffic lights, at x 20, and the 18 after them.
double _trafficLightsEnd(StormTokens t) => t.sp * 11.5;

/// The full-width bar over the rail, the sidebar and the pane (decision 153).
///
/// [full] adds back, forward and the health dot; without it the bar is only
/// the window's frame, for screens outside the signed-in shell and for a
/// desktop window narrower than the breakpoint.
///
/// Three regions: the window's buttons and back/forward, the search field
/// centred in the middle (156n), and the health dot with the caption
/// buttons. The blank bar either side of the field still moves the window.
class StormAppBar extends ConsumerWidget {
  const StormAppBar({super.key, this.full = true});

  final bool full;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final chrome = windowChrome;
    final window = ref.watch(windowStateProvider);
    final layout = chrome == WindowChrome.caption
        ? ref.watch(captionLayoutProvider).value ?? kDefaultCaptionLayout
        : (left: const <CaptionButton>[], right: const <CaptionButton>[]);
    final edge = t.sp * 2.5;
    final lead = switch (chrome) {
      WindowChrome.mac when !window.fullScreen => _trafficLightsEnd(t),
      _ when layout.left.isNotEmpty => t.sp * 1.25,
      _ => edge,
    };
    final controls = ref.read(windowStateProvider.notifier);

    // The line is under the 44, so the caption buttons fill all of it.
    return Column(
      key: const Key('app-bar'),
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          height: appBarHeight(t),
          color: t.bg,
          child: Stack(
            children: [
              // Under the buttons, so only blank bar moves the window.
              if (chrome != WindowChrome.none)
                Positioned.fill(
                  child: GestureDetector(
                    key: const Key('app-bar-drag'),
                    behavior: HitTestBehavior.opaque,
                    onPanStart: (_) => controls.drag(),
                    onDoubleTap: controls.toggleMaximized,
                  ),
                ),
              // Centres the field on the whole bar, sliding it aside only
              // when the ends would overlap it.
              NavigationToolbar(
                middleSpacing: t.sp * 2,
                leading: Row(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    if (layout.left.isNotEmpty)
                      CaptionButtons(buttons: layout.left),
                    Padding(
                      // Content sits 2px below centre.
                      padding: EdgeInsets.only(top: t.sp * 0.25),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          SizedBox(width: lead),
                          if (full) _NavButtons(focused: window.focused),
                        ],
                      ),
                    ),
                  ],
                ),
                middle: full
                    ? Padding(
                        padding: EdgeInsets.only(top: t.sp * 0.25),
                        child: BarSearchField(focused: window.focused),
                      )
                    : null,
                trailing: Row(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Padding(
                      padding: EdgeInsets.only(top: t.sp * 0.25),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          if (full) const BarHealthDot(),
                          SizedBox(
                            width: layout.right.isEmpty
                                ? t.sp * 0.75
                                : t.sp * 0.5,
                          ),
                        ],
                      ),
                    ),
                    if (layout.right.isNotEmpty)
                      CaptionButtons(buttons: layout.right),
                  ],
                ),
              ),
            ],
          ),
        ),
        Container(height: t.bw, color: t.border),
      ],
    );
  }
}

/// The bar alone, over a screen outside the shell, where the OS draws no
/// frame of its own.
class WindowFrame extends StatelessWidget {
  const WindowFrame({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (windowChrome == WindowChrome.none) return child;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const StormAppBar(full: false),
        Expanded(child: child),
      ],
    );
  }
}

/// The bar's search (156n, restoring 153b): it looks like a field and
/// opens the palette. Not a text field, so there is one query, the
/// palette's.
class BarSearchField extends ConsumerWidget {
  const BarSearchField({super.key, required this.focused});

  final bool focused;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final ink = focused ? t.text2 : t.text3;
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: ink,
    );
    final radius = BorderRadius.circular(t.rControl);
    // The palette's hints are hidden on the web, where browsers keep chords.
    final chord = stormHasKeyboard && !kIsWeb ? stormChordLabel('K') : null;
    return LayoutBuilder(
      builder: (context, constraints) {
        if (constraints.maxWidth < t.sp * 35) return const SizedBox.shrink();
        return SizedBox(
          width: t.sp * 52.5,
          height: t.sp * 3.5,
          child: Semantics(
            button: true,
            label: 'Search notes, sessions, commands',
            hint: chord,
            excludeSemantics: true,
            onTap: () => openPalette(context, ref),
            child: Material(
              color: t.surface,
              shape: RoundedRectangleBorder(
                borderRadius: radius,
                side: BorderSide(color: t.border, width: t.bw),
              ),
              child: InkWell(
                key: const Key('app-bar-search'),
                borderRadius: radius,
                canRequestFocus: false,
                onTap: () => openPalette(context, ref),
                child: Padding(
                  padding: EdgeInsets.symmetric(horizontal: t.sp * 1.25),
                  child: Row(
                    children: [
                      Icon(LucideIcons.search, size: t.uiSize, color: ink),
                      SizedBox(width: t.sp),
                      Expanded(
                        child: Text(
                          'Search notes, sessions, commands…',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: style,
                        ),
                      ),
                      if (chord != null)
                        Text(
                          chord,
                          style: style.copyWith(
                            fontFamily: StormTokens.monoFamily,
                            fontSize: t.labelSize,
                          ),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

class _NavButtons extends ConsumerWidget {
  const _NavButtons({required this.focused});

  final bool focused;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final history = ref.watch(navHistoryProvider);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        _NavButton(
          key: const Key('app-bar-back'),
          icon: LucideIcons.chevron_left,
          label: 'Back',
          enabled: history.canBack,
          focused: focused,
          onTap: () => goBack(context, ref),
        ),
        _NavButton(
          key: const Key('app-bar-forward'),
          icon: LucideIcons.chevron_right,
          label: 'Forward',
          enabled: history.canForward,
          focused: focused,
          onTap: () => goForward(context, ref),
        ),
      ],
    );
  }
}

void goBack(BuildContext context, WidgetRef ref) {
  final to = ref.read(navHistoryProvider.notifier).back();
  if (to != null) GoRouter.of(context).go(to);
}

void goForward(BuildContext context, WidgetRef ref) {
  final to = ref.read(navHistoryProvider.notifier).forward();
  if (to != null) GoRouter.of(context).go(to);
}

class _NavButton extends StatelessWidget {
  const _NavButton({
    super.key,
    required this.icon,
    required this.label,
    required this.enabled,
    required this.focused,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final bool enabled;
  final bool focused;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final ink = !enabled
        ? t.text3.withValues(alpha: 0.5)
        : (focused ? t.text2 : t.text3);
    final radius = BorderRadius.circular(t.rControl);
    return Tooltip(
      message: label,
      child: Semantics(
        button: true,
        enabled: enabled,
        label: label,
        excludeSemantics: true,
        onTap: enabled ? onTap : null,
        child: InkWell(
          borderRadius: radius,
          onTap: enabled ? onTap : null,
          child: SizedBox(
            width: t.sp * 3,
            height: t.sp * 3.5,
            child: Icon(icon, size: t.uiSize, color: ink),
          ),
        ),
      ),
    );
  }
}

/// The 9px health dot at the bar's right end, opening [HealthPopover]. It
/// moved here from the rail (153a).
class BarHealthDot extends ConsumerStatefulWidget {
  const BarHealthDot({super.key});

  @override
  ConsumerState<BarHealthDot> createState() => _BarHealthDotState();
}

class _BarHealthDotState extends ConsumerState<BarHealthDot> {
  final _anchor = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final tone = ref.watch(healthToneProvider);
    return Semantics(
      button: true,
      label: 'Status',
      onTap: _open,
      excludeSemantics: true,
      child: InkWell(
        key: const Key('bar-status'),
        borderRadius: BorderRadius.circular(t.rControl),
        onTap: _open,
        child: SizedBox.square(
          key: _anchor,
          dimension: t.sp * 3.5,
          child: Center(
            child: Container(
              width: t.sp * 1.125,
              height: t.sp * 1.125,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: healthColor(tone, t),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Future<void> _open() async {
    final t = context.tokens;
    await showStormPopover<void>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 35,
      alignRight: true,
      builder: (popContext) => HealthPopover(
        onAbout: () {
          Navigator.pop(popContext);
          context.go('/settings/health');
        },
      ),
    );
  }
}

Color healthColor(HealthTone tone, StormTokens t) => switch (tone) {
  HealthTone.good => t.green,
  HealthTone.muted => t.text3,
  HealthTone.danger => t.danger,
};

class HealthPopover extends StatelessWidget {
  const HealthPopover({super.key, required this.onAbout});

  final VoidCallback onAbout;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return StormPopover(
      children: [
        const HealthRows(),
        InkWell(
          key: const Key('health-about'),
          onTap: onAbout,
          borderRadius: BorderRadius.circular(t.rControl),
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 0.75,
              vertical: t.sp,
            ),
            child: Text(
              'About & health ›',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.accent,
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class HealthRows extends ConsumerWidget {
  const HealthRows({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final row in ref.watch(healthRowsProvider))
          Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 0.75,
              vertical: t.sp * 0.75,
            ),
            child: Row(
              children: [
                Container(
                  width: t.sp,
                  height: t.sp,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: healthColor(row.tone, t),
                  ),
                ),
                SizedBox(width: t.sp * 1.5),
                Expanded(
                  child: Text(
                    row.text,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: t.text,
                    ),
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}
