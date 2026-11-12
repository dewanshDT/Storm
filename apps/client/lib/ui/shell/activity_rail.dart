import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:go_router/go_router.dart';

import '../../agent/agent_state.dart';
import '../../state/nav_memory.dart';
import '../tokens.dart';
import 'app_bar.dart' show railTopInset;

/// The desktop activity rail (handoff §2.1): Notes, Agents, then Settings at
/// the foot. The health dot is in the app bar (153a).
class ActivityRail extends ConsumerWidget {
  const ActivityRail({super.key, required this.current});

  final Activity? current;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final memory = ref.watch(navMemoryProvider);
    // The card's status dot pulses for the same sessions (decision 123c).
    final working =
        ref
            .watch(agentOverviewProvider)
            .value
            ?.live
            .any((s) => s.status == 'running' && s.working) ??
        false;

    void open(Activity a) => context.go(memory.entryOf(a));

    return Container(
      width: t.sp * 7,
      decoration: BoxDecoration(
        color: t.bg,
        border: Border(
          right: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      padding: EdgeInsets.only(top: railTopInset(t), bottom: t.sp),
      child: SafeArea(
        right: false,
        child: Column(
          spacing: t.sp,
          children: [
            RailItem(
              key: const Key('rail-notes'),
              pixel: PixelGlyph.notes,
              label: 'Notes',
              active: current == Activity.notes,
              onTap: () => open(Activity.notes),
            ),
            RailItem(
              key: const Key('rail-agents'),
              pixel: PixelGlyph.agents,
              label: 'Agents',
              active: current == Activity.agents,
              animate: working,
              onTap: () => open(Activity.agents),
            ),
            const Spacer(),
            RailItem(
              key: const Key('rail-settings'),
              icon: LucideIcons.settings_2,
              label: 'Settings',
              active: current == Activity.settings,
              onTap: () => open(Activity.settings),
            ),
          ],
        ),
      ),
    );
  }
}

/// A pixel-art glyph from `assets/rail/`: one colour, tinted like an icon,
/// and drawn on whole pixels so its cells stay crisp.
class PixelGlyph {
  const PixelGlyph._(this.frames, this.columns, this.rows);

  static const notes = PixelGlyph._(['assets/rail/notes.svg'], 10, 11);
  static const agents = PixelGlyph._(
    ['assets/rail/agents.svg', 'assets/rail/agents_alt.svg'],
    11,
    8,
  );

  /// The still frame first; the rest alternate with it while animating.
  final List<String> frames;
  final int columns;
  final int rows;
}

/// One frame swap per half of the card's pulse (1100 ms), so the two keep
/// time.
const _frameTime = Duration(milliseconds: 550);

/// One square rail button, glyph centred, no label: the label is its tooltip
/// and its name for screen readers.
class RailItem extends StatelessWidget {
  const RailItem({
    super.key,
    this.icon,
    this.pixel,
    required this.label,
    required this.active,
    required this.onTap,
    this.animate = false,
  }) : assert((icon == null) != (pixel == null));

  final IconData? icon;
  final PixelGlyph? pixel;
  final String label;
  final bool active;
  final VoidCallback onTap;

  /// Cycle a [pixel] glyph's frames (decision 123c).
  final bool animate;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final ink = active ? t.accent : t.text3;
    final radius = BorderRadius.circular(t.rControl);
    return Tooltip(
      message: label,
      child: Semantics(
        button: true,
        selected: active,
        label: label,
        onTap: onTap,
        excludeSemantics: true,
        child: Material(
          color: active ? t.accentSoft : Colors.transparent,
          borderRadius: radius,
          child: InkWell(
            onTap: onTap,
            borderRadius: radius,
            child: SizedBox.square(
              dimension: railFootSize(t),
              child: Center(child: _glyph(t, ink)),
            ),
          ),
        ),
      ),
    );
  }

  Widget _glyph(StormTokens t, Color ink) {
    final pixel = this.pixel;
    if (pixel == null) return Icon(icon, size: t.sp * 2.5, color: ink);
    return _PixelGlyphView(glyph: pixel, ink: ink, animate: animate);
  }
}

class _PixelGlyphView extends StatefulWidget {
  const _PixelGlyphView({
    required this.glyph,
    required this.ink,
    required this.animate,
  });

  final PixelGlyph glyph;
  final Color ink;
  final bool animate;

  @override
  State<_PixelGlyphView> createState() => _PixelGlyphViewState();
}

class _PixelGlyphViewState extends State<_PixelGlyphView>
    with SingleTickerProviderStateMixin {
  late final _cycle = AnimationController(
    vsync: this,
    duration: _frameTime * widget.glyph.frames.length,
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _sync();
  }

  @override
  void didUpdateWidget(_PixelGlyphView old) {
    super.didUpdateWidget(old);
    _sync();
  }

  void _sync() {
    final run =
        widget.animate &&
        widget.glyph.frames.length > 1 &&
        !MediaQuery.disableAnimationsOf(context);
    if (run && !_cycle.isAnimating) {
      _cycle.repeat();
    } else if (!run && _cycle.isAnimating) {
      _cycle
        ..stop()
        ..value = 0;
    }
  }

  @override
  void dispose() {
    _cycle.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final glyph = widget.glyph;
    // Whole logical pixels per cell, so the art never lands between them.
    final cell = (t.sp * 0.25).roundToDouble().clamp(1.0, double.infinity);
    final frames = [
      for (final asset in glyph.frames)
        SvgPicture.asset(
          asset,
          width: glyph.columns * cell,
          height: glyph.rows * cell,
          colorFilter: ColorFilter.mode(widget.ink, BlendMode.srcIn),
        ),
    ];
    if (frames.length == 1) return frames.single;
    // Every frame stays built, so a swap never waits on an asset load.
    return AnimatedBuilder(
      animation: _cycle,
      builder: (context, _) => IndexedStack(
        key: const Key('rail-glyph-frames'),
        index: (_cycle.value * frames.length).floor() % frames.length,
        children: frames,
      ),
    );
  }
}

double railFootSize(StormTokens t) => t.sp * 5;
