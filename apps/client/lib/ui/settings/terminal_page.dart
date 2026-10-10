import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../agent/terminal_surface.dart';
import '../../state/terminal_prefs.dart';
import '../breakpoints.dart';
import '../controls.dart';
import '../tokens.dart';
import 'settings_widgets.dart';

const _sample =
    '\x1b[2m~/storm\x1b[0m \x1b[36m❯\x1b[0m claude\r\n'
    '\r\n'
    '\x1b[38;2;217;119;87m✻\x1b[0m Welcome back\r\n'
    '╭────────────────────────────╮\r\n'
    '│ > fix the login redirect   │\r\n'
    '╰────────────────────────────╯\r\n'
    '\x1b[1m●\x1b[0m Read \x1b[33msrc/auth.rs\x1b[0m\r\n'
    '  The redirect drops the query string.\r\n'
    '\x1b[32m+\x1b[0m   let next = url.query().unwrap_or("");\r\n'
    '\x1b[2m  2 lines changed · 3s\x1b[0m';

/// Settings › Terminal: client preferences for every terminal in this app
/// (D15 AM47).
class TerminalPage extends ConsumerStatefulWidget {
  const TerminalPage({super.key});

  @override
  ConsumerState<TerminalPage> createState() => _TerminalPageState();
}

class _TerminalPageState extends ConsumerState<TerminalPage> {
  final _preview = StormTerminal();

  @override
  void initState() {
    super.initState();
    _preview.write(Uint8List.fromList(utf8.encode(_sample)));
  }

  @override
  void dispose() {
    _preview.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final prefs = ref.watch(terminalPrefsProvider);
    final notifier = ref.read(terminalPrefsProvider.notifier);
    final phone = !context.isExpanded;
    final layoutSize = (phone ? t.labelSize + 1 : t.codeSize).roundToDouble();

    return SettingsPage(
      title: 'Terminal',
      intro: 'Every agent session in this app. Kept on this device.',
      children: [
        SettingsRow(
          label: 'Text size',
          trailing: _SizeStepper(
            value: prefs.fontSize,
            fallback: layoutSize,
            onChanged: (v) => notifier.set(prefs.copyWith(fontSize: v)),
          ),
        ),
        GroupLabel('Line spacing', top: t.sp * 2.5, bottom: t.sp),
        ChoiceChips<TerminalSpacing>(
          key: const Key('terminal-spacing'),
          options: [
            for (final s in TerminalSpacing.values) ChoiceOption(s, s.label),
          ],
          selected: prefs.spacing,
          onSelected: (s) => notifier.set(prefs.copyWith(spacing: s)),
        ),
        GroupLabel('Padding', top: t.sp * 2.5, bottom: t.sp),
        ChoiceChips<TerminalPadding>(
          key: const Key('terminal-padding'),
          options: [
            for (final p in TerminalPadding.values) ChoiceOption(p, p.label),
          ],
          selected: prefs.padding,
          onSelected: (p) => notifier.set(prefs.copyWith(padding: p)),
        ),
        GroupLabel('Preview', top: t.sp * 2.5, bottom: t.sp),
        ClipRRect(
          borderRadius: BorderRadius.circular(t.rControl),
          child: DecoratedBox(
            position: DecorationPosition.foreground,
            decoration: BoxDecoration(
              border: Border.all(color: t.border, width: t.bw),
              borderRadius: BorderRadius.circular(t.rControl),
            ),
            child: SizedBox(
              key: const Key('terminal-preview'),
              height: t.sp * 30,
              child: IgnorePointer(
                child: StormTerminalView(
                  terminal: _preview,
                  readOnly: true,
                  phone: phone,
                  prefs: prefs,
                ),
              ),
            ),
          ),
        ),
        if (!prefs.isDefault)
          SettingsButtonRow(
            top: t.sp * 1.5,
            child: TextAction(
              key: const Key('terminal-reset'),
              label: 'Reset to defaults',
              onTap: notifier.reset,
            ),
          ),
      ],
    );
  }
}

class _SizeStepper extends StatelessWidget {
  const _SizeStepper({
    required this.value,
    required this.fallback,
    required this.onChanged,
  });

  final double? value;
  final double fallback;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final current = value ?? fallback;
    Widget step(String label, IconData icon, double next, Key key) {
      final live =
          next >= TerminalPrefs.minFontSize &&
          next <= TerminalPrefs.maxFontSize;
      return Semantics(
        button: true,
        label: label,
        onTap: live ? () => onChanged(next) : null,
        excludeSemantics: true,
        child: InkWell(
          key: key,
          onTap: live ? () => onChanged(next) : null,
          borderRadius: BorderRadius.circular(t.rControl * 0.6),
          child: Padding(
            padding: EdgeInsets.all(t.sp * 0.5),
            child: Icon(
              icon,
              size: t.codeSize,
              color: live ? t.text2 : t.border,
            ),
          ),
        ),
      );
    }

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        step(
          'Smaller terminal text',
          LucideIcons.minus,
          current - 1,
          const Key('terminal-smaller'),
        ),
        SizedBox(width: t.sp * 0.5),
        MonoValue(
          value == null ? 'Default' : '${current.round()} px',
          key: const Key('terminal-size'),
        ),
        SizedBox(width: t.sp * 0.5),
        step(
          'Larger terminal text',
          LucideIcons.plus,
          current + 1,
          const Key('terminal-larger'),
        ),
      ],
    );
  }
}
