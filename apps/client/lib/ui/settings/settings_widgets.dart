/// The pieces every settings page is built from (handoff §5.2, §5.3).
library;

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../breakpoints.dart';
import '../shell/storm_scaffold.dart';
import '../tokens.dart';

/// A page's frame: the 680 column beside the navigation at desk width; on a
/// phone, a pushed screen under the corner bubbles with "‹ Settings" first.
class SettingsPage extends StatelessWidget {
  const SettingsPage({
    super.key,
    required this.title,
    required this.intro,
    required this.children,
  });

  final String title;
  final String intro;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final wide = context.isExpanded;
    final body = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (!wide) ...[const _BackToSettings(), SizedBox(height: t.sp * 2)],
        Text(
          title,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.titleSize,
            fontWeight: FontWeight.w600,
            color: t.text,
            height: 1.2,
          ),
        ),
        SizedBox(height: t.sp * 1.25),
        Text(
          intro,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.uiSize,
            color: t.text2,
            height: 1.5,
          ),
        ),
        SizedBox(height: wide ? t.sp * 2.25 : t.sp * 2),
        ...children,
      ],
    );

    if (!wide) {
      return StormScaffold(
        showNav: false,
        child: ListView(
          padding: EdgeInsets.fromLTRB(
            StormChrome.contentInset(context),
            0,
            StormChrome.contentInset(context),
            t.sp * 7.5,
          ),
          children: [body],
        ),
      );
    }
    return Material(
      color: t.bg,
      child: ListView(
        padding: EdgeInsets.fromLTRB(
          t.sp * 5,
          t.sp * 3.5,
          t.sp * 5,
          t.sp * 7.5,
        ),
        children: [
          Align(
            alignment: Alignment.topLeft,
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: t.sp * 75),
              child: body,
            ),
          ),
        ],
      ),
    );
  }
}

class _BackToSettings extends StatelessWidget {
  const _BackToSettings();

  @override
  Widget build(BuildContext context) => Align(
    alignment: Alignment.centerLeft,
    child: TextAction(
      key: const Key('settings-back'),
      label: '‹ Settings',
      accent: true,
      large: true,
      onTap: () =>
          context.canPop() ? context.pop() : context.go(Routes.settings),
    ),
  );
}

/// Mono 11, uppercase, `text3` (handoff §7.2).
class GroupLabel extends StatelessWidget {
  const GroupLabel(this.text, {super.key, this.top = 0, this.bottom});

  final String text;
  final double top;
  final double? bottom;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(top: top, bottom: bottom ?? t.sp * 0.5),
      child: Text(
        text.toUpperCase(),
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.labelSize,
          letterSpacing: t.labelSize * 0.08,
          color: t.text3,
        ),
      ),
    );
  }
}

/// 13 `text2`, or `accent` for navigation, or `danger`.
class TextAction extends StatelessWidget {
  const TextAction({
    super.key,
    required this.label,
    required this.onTap,
    this.accent = false,
    this.danger = false,
    this.large = false,
  });

  final String label;
  final VoidCallback? onTap;
  final bool accent;
  final bool danger;
  final bool large;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final color = danger
        ? t.danger
        : accent
        ? t.accent
        : t.text2;
    return Semantics(
      button: true,
      label: label,
      onTap: onTap,
      excludeSemantics: true,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl * 0.6),
        child: Padding(
          padding: EdgeInsets.symmetric(vertical: t.sp * 0.25),
          child: Text(
            label,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: large ? t.uiSize : t.codeSize,
              color: onTap == null ? t.text3 : color,
            ),
          ),
        ),
      ),
    );
  }
}

/// Text actions side by side, as on a vault or host row.
class RowActions extends StatelessWidget {
  const RowActions({super.key, required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    spacing: context.tokens.sp * 1.5,
    children: children,
  );
}

/// A value on the right of a row: an address, a version, a fingerprint.
class MonoValue extends StatelessWidget {
  const MonoValue(this.text, {super.key, this.color});

  final String text;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      text,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      textAlign: TextAlign.right,
      style: TextStyle(
        fontFamily: StormTokens.monoFamily,
        fontSize: t.labelSize * 1.09,
        color: color ?? t.text2,
      ),
    );
  }
}

/// 13 `text2` prose under a group label or after a list.
class SettingsNote extends StatelessWidget {
  const SettingsNote(this.text, {super.key, this.small = false});

  final String text;

  /// The 12 `text3` footnote size.
  final bool small;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      text,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: small ? t.labelSize * 1.09 : t.codeSize,
        color: small ? t.text3 : t.text2,
        height: 1.5,
      ),
    );
  }
}

/// `surface`, border, `rControl` (handoff §5.3 Integrations).
class InfoBox extends StatelessWidget {
  const InfoBox(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 1.75,
        vertical: t.sp * 1.5,
      ),
      decoration: BoxDecoration(
        color: t.surface,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.codeSize,
          color: t.text2,
          height: 1.55,
        ),
      ),
    );
  }
}

/// Loading and failure inside a page, in the page's own voice.
class SettingsMuted extends StatelessWidget {
  const SettingsMuted(this.text, {super.key, this.danger = false});

  final String text;
  final bool danger;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.symmetric(vertical: t.sp * 1.5),
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.uiSize,
          color: danger ? t.danger : t.text3,
        ),
      ),
    );
  }
}

/// The left-aligned button after a list.
class SettingsButtonRow extends StatelessWidget {
  const SettingsButtonRow({super.key, required this.child, this.top});

  final Widget child;
  final double? top;

  @override
  Widget build(BuildContext context) => Padding(
    padding: EdgeInsets.only(top: top ?? context.tokens.sp * 1.5),
    child: Align(alignment: Alignment.centerLeft, child: child),
  );
}

void settingsToast(BuildContext context, String message) =>
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));

/// A yes/no question before something that cannot be taken back.
Future<bool> confirmSetting(
  BuildContext context, {
  required String title,
  required String body,
  required String action,
  Key? confirmKey,
}) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (c) => AlertDialog(
      title: Text(title),
      content: Text(body),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(c, false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: confirmKey,
          onPressed: () => Navigator.pop(c, true),
          child: Text(action),
        ),
      ],
    ),
  );
  return ok == true;
}

/// A one-field prompt.
///
/// A `StatefulWidget` so the controller outlives the closing animation;
/// disposing it while the route animates out produced a red screen once.
Future<String?> promptForText(
  BuildContext context, {
  required String title,
  required String hint,
  String initial = '',
  String? label,
  String? help,
}) => showDialog<String>(
  context: context,
  builder: (_) => _TextDialog(
    title: title,
    hint: hint,
    initial: initial,
    label: label,
    help: help,
  ),
);

class _TextDialog extends StatefulWidget {
  const _TextDialog({
    required this.title,
    required this.hint,
    required this.initial,
    this.label,
    this.help,
  });

  final String title;
  final String hint;
  final String initial;
  final String? label;
  final String? help;

  @override
  State<_TextDialog> createState() => _TextDialogState();
}

class _TextDialogState extends State<_TextDialog> {
  late final _controller = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _submit() => Navigator.pop(context, _controller.text);

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(widget.title),
    content: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (widget.help != null) ...[
          Text(widget.help!),
          SizedBox(height: context.tokens.sp * 1.5),
        ],
        TextField(
          key: const Key('prompt-field'),
          controller: _controller,
          autofocus: true,
          decoration: InputDecoration(
            labelText: widget.label,
            hintText: widget.hint,
          ),
          onSubmitted: (_) => _submit(),
        ),
      ],
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
      FilledButton(
        key: const Key('prompt-ok'),
        onPressed: _submit,
        child: const Text('OK'),
      ),
    ],
  );
}
