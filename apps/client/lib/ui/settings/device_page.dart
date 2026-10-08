import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../keyboard/storm_activators.dart';
import '../../state/app_state.dart';
import '../controls.dart';
import '../surfaces.dart';
import '../widgets.dart' show PopoverItem;
import '../tokens.dart';
import 'settings_widgets.dart';

const _minText = 12.0;
const _maxText = 24.0;

String _fontName(BodyFont f) => switch (f) {
  BodyFont.serif => 'Newsreader',
  BodyFont.sans => 'Sans',
  BodyFont.mono => 'Monospace',
};

/// Settings › This device: client preferences only (handoff §5.4).
class DevicePage extends ConsumerWidget {
  const DevicePage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final notifier = ref.read(settingsProvider.notifier);
    void save(Settings next) => notifier.save(next);

    return SettingsPage(
      title: 'This device',
      intro: 'Only affects this device.',
      children: [
        const GroupLabel('Appearance', bottom: 10),
        Wrap(
          spacing: t.sp,
          runSpacing: t.sp,
          children: [
            for (final preset in StormPreset.values)
              _PresetButton(
                preset: preset,
                selected: settings.theme == preset,
                onTap: () => save(settings.copyWith(theme: preset)),
              ),
          ],
        ),
        SizedBox(height: t.sp * 1.25),
        SettingsRow(
          label: 'Text size',
          trailing: _TextSizeStepper(
            value: settings.fontSize,
            onChanged: (v) => save(settings.copyWith(fontSize: v)),
          ),
        ),
        SizedBox(height: t.sp * 1.25),
        SettingsRow(
          label: 'Note font',
          trailing: _FontPicker(
            value: settings.bodyFont,
            onChanged: (f) => save(settings.copyWith(bodyFont: f)),
          ),
        ),
        GroupLabel('Notes', top: t.sp * 3),
        SizedBox(height: t.sp * 1.25),
        SettingsRow(
          label: 'Open notes in Read mode',
          sub: 'Switch to Edit with ${_mod()}E.',
          trailing: StormToggle(
            key: const Key('setting-read-mode'),
            value: settings.readMode,
            onChanged: (v) => save(settings.copyWith(readMode: v)),
          ),
        ),
        SizedBox(height: t.sp * 1.25),
        SettingsRow(
          label: 'Show note id',
          sub: 'Shown on the version line.',
          trailing: StormToggle(
            key: const Key('setting-note-id'),
            value: settings.showNoteId,
            onChanged: (v) => save(settings.copyWith(showNoteId: v)),
          ),
        ),
        GroupLabel('Keyboard', top: t.sp * 3, bottom: t.sp * 1.25),
        Wrap(
          spacing: t.sp * 2.25,
          runSpacing: t.sp,
          children: [
            _Shortcut('${_mod()}K', 'Search'),
            _Shortcut('${_mod()}N', 'New note'),
            _Shortcut('${_mod()}\\', 'Sidebar'),
            _Shortcut('${_mod()}E', 'Read / Edit'),
          ],
        ),
      ],
    );
  }
}

String _mod() => stormUsesMetaModifier ? '⌘' : 'Ctrl+';

class _PresetButton extends StatelessWidget {
  const _PresetButton({
    required this.preset,
    required this.selected,
    required this.onTap,
  });

  final StormPreset preset;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final own = StormTokens.from(preset);
    final radius = BorderRadius.circular(t.rControl);
    return Semantics(
      button: true,
      selected: selected,
      label: preset.label,
      onTap: onTap,
      excludeSemantics: true,
      child: Material(
        key: Key('preset-${preset.name}'),
        color: selected ? t.accentSoft : t.surface,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: BorderSide(color: selected ? t.accent : t.border, width: t.bw),
        ),
        child: InkWell(
          onTap: onTap,
          borderRadius: radius,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.5,
              vertical: t.sp,
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Container(
                  width: t.sp * 2.25,
                  height: t.sp * 2.25,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: own.bg,
                    border: Border.all(color: own.accent, width: t.bw * 3),
                  ),
                ),
                SizedBox(width: t.sp),
                Text(
                  preset.label,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _TextSizeStepper extends StatelessWidget {
  const _TextSizeStepper({required this.value, required this.onChanged});

  final double value;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    Widget step(String label, IconData icon, double next, Key key) {
      final live = next >= _minText && next <= _maxText;
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
          'Smaller text',
          LucideIcons.minus,
          value - 1,
          const Key('text-smaller'),
        ),
        SizedBox(width: t.sp * 0.5),
        MonoValue('${value.round()} px'),
        SizedBox(width: t.sp * 0.5),
        step(
          'Larger text',
          LucideIcons.plus,
          value + 1,
          const Key('text-larger'),
        ),
      ],
    );
  }
}

class _FontPicker extends StatefulWidget {
  const _FontPicker({required this.value, required this.onChanged});

  final BodyFont value;
  final ValueChanged<BodyFont> onChanged;

  @override
  State<_FontPicker> createState() => _FontPickerState();
}

class _FontPickerState extends State<_FontPicker> {
  final _anchor = GlobalKey();

  Future<void> _open() async {
    final t = context.tokens;
    final chosen = await showStormPopover<BodyFont>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 25,
      alignRight: true,
      builder: (pop) => StormPopover(
        children: [
          for (final f in BodyFont.values)
            PopoverItem(
              key: Key('font-${f.name}'),
              label: _fontName(f),
              trailing: f == widget.value
                  ? Icon(LucideIcons.check, size: t.uiSize, color: t.accent)
                  : null,
              onTap: () => Navigator.pop(pop, f),
            ),
        ],
      ),
    );
    if (chosen != null) widget.onChanged(chosen);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final name = _fontName(widget.value);
    return Semantics(
      button: true,
      label: 'Note font, $name',
      onTap: _open,
      excludeSemantics: true,
      child: InkWell(
        key: _anchor,
        onTap: _open,
        borderRadius: BorderRadius.circular(t.rControl * 0.6),
        child: Padding(
          padding: EdgeInsets.symmetric(vertical: t.sp * 0.25),
          child: Text(
            name,
            key: const Key('note-font'),
            style: TextStyle(
              fontFamily: widget.value.family,
              fontSize: t.codeSize * 1.17,
              color: t.text2,
            ),
          ),
        ),
      ),
    );
  }
}

class _Shortcut extends StatelessWidget {
  const _Shortcut(this.keys, this.label);

  final String keys;
  final String label;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text.rich(
      TextSpan(
        children: [
          TextSpan(
            text: keys,
            style: TextStyle(fontFamily: StormTokens.monoFamily, color: t.text),
          ),
          TextSpan(text: ' $label'),
        ],
      ),
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.codeSize,
        color: t.text2,
      ),
    );
  }
}
