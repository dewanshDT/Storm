import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart' show PopoverItem;
import 'agent_models.dart';
import 'agent_state.dart';
import 'crew_widgets.dart';

/// A session's crew and role where the session view shows them (decision
/// 138, handoff §3 Session, 4e): the chip and tag on the meta line, and the
/// Crew and Role rows in About with Move to crew and Change label.

/// The session's agent can read Storm notes (138g): by the host's word for
/// that agent, never by its name. A launch role means it could.
bool sessionReadsStorm(AgentOverview? o, AgentSession s) {
  if (s.launchRole != null) return true;
  final host = o?.hosts.where((h) => h.id == s.hostId).firstOrNull;
  final cap = host?.providers.where((p) => p.id == s.provider).firstOrNull;
  return cap?.reachesStorm ?? !s.isShell;
}

String _tag(RoleRef r, {RoleRef? launch}) {
  final text = roleTagText(r, launchRole: launch);
  return r.deleted ? '$text (its note was deleted)' : text;
}

/// About's Role line (138f, 138g): the launch role the agent read and the
/// label, both when they differ.
String aboutRoleText(AgentSession s, {required bool readsStorm}) {
  final launch = s.launchRole, label = s.role;
  if (launch == null) {
    if (label == null) return 'None';
    final tag = 'Label: ${_tag(label)}.';
    return readsStorm
        ? "$tag Set after launch: the agent didn't read this note."
        : "$tag ${providerLabel(s.provider)} can't read Storm notes, so this "
              'is only a label.';
  }
  final launched =
      'Launched as ${(launch.title ?? 'a role').toLowerCase()} (the agent '
      'read this note)';
  if (label == null) return '$launched · No label';
  if (label.sameNote(launch)) {
    return label.deleted
        ? '$launched. Its note has since been deleted.'
        : launched;
  }
  return '$launched · Label: ${_tag(label, launch: launch)}';
}

/// The meta line's crew chip and role tag.
class SessionMetaTags extends ConsumerWidget {
  const SessionMetaTags({super.key, required this.session});

  final AgentSession session;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final crew = ref.watch(agentOverviewProvider).value?.crewOf(session);
    final role = session.role;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (crew != null) ...[
          SizedBox(width: t.sp),
          Semantics(
            button: true,
            label: 'Open crew ${crew.name}',
            excludeSemantics: true,
            onTap: () => context.go(Routes.agentCrew(crew.id)),
            child: InkWell(
              key: const Key('session-crew-chip'),
              borderRadius: BorderRadius.circular(t.rControl),
              onTap: () => context.go(Routes.agentCrew(crew.id)),
              child: Container(
                padding: EdgeInsets.symmetric(
                  horizontal: t.sp,
                  vertical: t.sp * 0.25,
                ),
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(t.rControl),
                  border: Border.all(color: t.border, width: t.bw),
                ),
                child: CrewChip(crew: crew),
              ),
            ),
          ),
        ],
        if (role != null) ...[
          SizedBox(width: t.sp),
          RoleTag(role: role, launchRole: session.launchRole),
        ],
      ],
    );
  }
}

/// About's Crew and Role rows, in the shape of its other rows.
class AboutCrewRole extends ConsumerStatefulWidget {
  const AboutCrewRole({super.key, required this.session});

  final AgentSession session;

  @override
  ConsumerState<AboutCrewRole> createState() => _AboutCrewRoleState();
}

class _AboutCrewRoleState extends ConsumerState<AboutCrewRole> {
  final _crewAnchor = GlobalKey();
  final _roleAnchor = GlobalKey();

  AgentSession get _s => widget.session;

  void _toast(String text) => ScaffoldMessenger.maybeOf(
    context,
  )?.showSnackBar(SnackBar(content: Text(text)));

  void _reload() {
    ref.invalidate(agentOverviewProvider);
    ref.invalidate(sessionDetailProvider(_s.id));
  }

  Future<void> _move(List<Crew> crews) async {
    final t = context.tokens;
    final current = _s.crew?.id;
    final picked = await showStormPopover<(String?,)>(
      context: context,
      anchorKey: _crewAnchor,
      width: t.sp * 30,
      builder: (popover) => StormPopover(
        key: const Key('move-to-crew-menu'),
        padding: t.sp * 0.75,
        children: [
          PopoverItem(
            key: const Key('move-to-none'),
            label: 'Not in a crew',
            selected: current == null,
            onTap: () => Navigator.of(popover).pop((null,)),
          ),
          for (final c in crews)
            PopoverItem(
              key: Key('move-to-${c.id}'),
              label: c.name,
              leading: CrewSquare(accent: c.accent, size: t.sp * 1.25),
              selected: c.id == current,
              onTap: () => Navigator.of(popover).pop((c.id,)),
            ),
        ],
      ),
    );
    if (picked == null || picked.$1 == current || !mounted) return;
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final change = await api.moveToCrew(_s.id, picked.$1);
      for (final c in change.removedCrews) {
        _toast('${c.name} was removed: its last session left it.');
      }
    } catch (e) {
      _toast("Couldn't move ${_s.displayName}: ${describeFailure(e)}");
    } finally {
      api.dispose();
      _reload();
    }
  }

  Future<void> _relabel() async {
    final t = context.tokens;
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final roles = await api.roles();
      if (!mounted) return;
      final label = _s.role;
      final picked = await showStormPopover<(RoleNote?,)>(
        context: context,
        anchorKey: _roleAnchor,
        width: t.sp * 40,
        builder: (popover) => RoleMenu(
          roles: roles,
          room: popoverRoom(context, _roleAnchor),
          selected: label == null
              ? null
              : (vaultId: label.vaultId, noteId: label.noteId),
          noneLabel: 'No label',
          noneDetail: _s.launchRole == null
              ? 'No role'
              : 'The launch role stays what the agent read',
          onPick: (r) => Navigator.of(popover).pop((r,)),
        ),
      );
      if (picked == null || !mounted) return;
      final r = picked.$1;
      if (r == null) {
        if (label == null) return;
        await api.clearRole(_s.id);
      } else {
        if (label != null && label.noteId == r.noteId) return;
        await api.setRole(_s.id, r.vaultId, r.noteId);
      }
    } catch (e) {
      _toast("Couldn't change the label: ${describeFailure(e)}");
    } finally {
      api.dispose();
      _reload();
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;
    final crews = o?.crews;
    final crew = o?.crewOf(_s);
    final value = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.uiSize,
      height: 1.45,
      color: t.text,
    );

    Widget row(String key, Widget body) => Padding(
      padding: EdgeInsets.only(top: t.sp * 1.75),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            key.toUpperCase(),
            style: TextStyle(
              fontFamily: StormTokens.monoFamily,
              fontSize: t.labelSize,
              letterSpacing: t.labelSize * 0.08,
              color: t.text3,
            ),
          ),
          SizedBox(height: t.sp * 0.375),
          body,
        ],
      ),
    );

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // A server before decision 138 has no crews to move between.
        if (crews != null)
          row(
            'Crew',
            Column(
              key: const Key('about-crew'),
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(crew?.name ?? 'Not in a crew', style: value),
                _AboutAction(
                  key: const Key('move-to-crew'),
                  anchor: _crewAnchor,
                  label: 'Move to crew',
                  onTap: () => _move(crews),
                ),
              ],
            ),
          ),
        row(
          'Role',
          Column(
            key: const Key('about-role'),
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                aboutRoleText(_s, readsStorm: sessionReadsStorm(o, _s)),
                key: const Key('about-role-text'),
                style: value,
              ),
              _AboutAction(
                key: const Key('change-label'),
                anchor: _roleAnchor,
                label: 'Change label',
                onTap: _relabel,
              ),
            ],
          ),
        ),
      ],
    );
  }
}

/// "Move to crew ▾": an accent text action that opens a menu under it.
class _AboutAction extends StatelessWidget {
  const _AboutAction({
    super.key,
    required this.anchor,
    required this.label,
    required this.onTap,
  });

  final GlobalKey anchor;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      button: true,
      label: label,
      excludeSemantics: true,
      onTap: onTap,
      child: InkWell(
        key: anchor,
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl * 0.6),
        child: Padding(
          padding: EdgeInsets.symmetric(vertical: t.sp * 0.5),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                label,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: t.accent,
                ),
              ),
              SizedBox(width: t.sp * 0.5),
              Icon(
                LucideIcons.chevron_down,
                size: t.labelSize,
                color: t.accent,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
