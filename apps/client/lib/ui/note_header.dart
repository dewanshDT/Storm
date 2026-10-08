import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../agent/agent_state.dart' show agentOverviewProvider;
import '../agent/agents_screen.dart' show launchAgentSession;
import '../router.dart';
import 'controls.dart';
import 'tokens.dart';
import 'widgets.dart' show SaveTone;

/// The note header's soft action. Opens the launcher while a host is online,
/// otherwise Agents, which explains what is missing.
class StartSessionButton extends ConsumerWidget {
  const StartSessionButton({super.key, this.compact = false});

  /// The phone's shorter "Session".
  final bool compact;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return StormButton.soft(
      key: const Key('start-session'),
      label: compact ? 'Session' : 'Start session',
      icon: Icons.play_arrow_rounded,
      onPressed: () async {
        final overview = ref.read(agentOverviewProvider).value;
        if (overview == null || overview.online.isEmpty) {
          context.go(Routes.agents);
          return;
        }
        final launched = await launchAgentSession(context, ref);
        if (launched != null && context.mounted) context.go(Routes.agents);
      },
    );
  }
}

/// `vault / folder / folder`, the desktop note header's crumb.
class NoteCrumb extends StatelessWidget {
  const NoteCrumb({super.key, required this.parts});

  final List<String> parts;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      parts.where((p) => p.isNotEmpty).join(' / '),
      key: const Key('note-crumb'),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: TextStyle(
        fontFamily: StormTokens.monoFamily,
        fontSize: t.codeSize,
        color: t.text3,
      ),
    );
  }
}

/// The drawer toggle at the end of the desktop note header.
class DrawerToggle extends StatelessWidget {
  const DrawerToggle({super.key, required this.open, required this.onTap});

  final bool open;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Tooltip(
      message: 'Properties',
      child: InkWell(
        key: const Key('drawer-toggle'),
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl),
        child: Padding(
          padding: EdgeInsets.all(t.sp * 0.75),
          child: Icon(
            LucideIcons.panel_right,
            size: t.bodySize * 1.0625,
            color: open ? t.text2 : t.text3,
          ),
        ),
      ),
    );
  }
}

/// `v51 · Saved`, the note id when asked for, and the slot slice 8 fills
/// with the provenance link.
class VersionLine extends StatelessWidget {
  const VersionLine({
    super.key,
    required this.version,
    required this.label,
    required this.tone,
    this.noteId,
    this.error,
    this.provenance,
  });

  final int version;
  final String label;
  final SaveTone tone;
  final String? noteId;
  final String? error;
  final Widget? provenance;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final mono = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.codeSize,
      color: t.text3,
    );
    final stateColor = switch (tone) {
      SaveTone.good || SaveTone.working => t.text3,
      SaveTone.waiting => t.amber,
      SaveTone.bad => t.danger,
    };
    return Wrap(
      spacing: t.sp * 1.25,
      runSpacing: t.sp * 0.75,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        Text.rich(
          TextSpan(
            style: mono,
            children: [
              TextSpan(text: 'v$version'),
              if (label.isNotEmpty) ...[
                const TextSpan(text: ' · '),
                TextSpan(
                  text: label,
                  style: TextStyle(color: stateColor),
                ),
              ],
              if (noteId != null) TextSpan(text: ' · id $noteId'),
            ],
          ),
          key: const Key('version-line'),
        ),
        ?provenance,
        if (error != null)
          Text(
            error!,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: mono.copyWith(color: t.danger),
          ),
      ],
    );
  }
}

/// The note's file name as its title, unless the body opens with its own
/// `# heading` — then that heading is the title and this would repeat it.
String? displayTitleFor(String path, String body) {
  final first = body
      .split('\n')
      .map((l) => l.trim())
      .firstWhere((l) => l.isNotEmpty, orElse: () => '');
  if (first.startsWith('# ')) return null;
  final name = path.split('/').last;
  return name.endsWith('.md') ? name.substring(0, name.length - 3) : name;
}
