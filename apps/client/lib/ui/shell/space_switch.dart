import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import '../tokens.dart';
import 'dashboard.dart' show notesHome;

/// The two spaces a wide sidebar can show.
enum StormSpace { notes, agents }

/// **Notes | Agents**, atop the wide sidebar (decision 78).
///
/// Gates itself on the server's owner check, so a sidebar can place it
/// unconditionally and an account that is not an owner sees nothing at all —
/// not a disabled control, which would advertise a feature it cannot have
/// (AC-S1).
///
/// "Notes" rather than "Vaults": each side names what you do there. Drawn in
/// `NoteModeToggle`'s language — the same surface, radii and inactive ink —
/// at the width of the rail rather than the width of its labels.
class SpaceSwitch extends ConsumerWidget {
  const SpaceSwitch({super.key, required this.current});

  final StormSpace current;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;

    Widget segment(StormSpace space, String label) {
      final on = space == current;
      return Expanded(
        child: Semantics(
          selected: on,
          button: true,
          child: InkWell(
            key: Key('space-${space.name}'),
            borderRadius: BorderRadius.circular(t.rControl * 0.7),
            // Selecting the space you are in does nothing, not a navigation.
            onTap: on
                ? null
                : () => context.go(switch (space) {
                    // The vault last in use — see [notesHome] for why not
                    // the dashboard.
                    StormSpace.notes => notesHome(
                      ref.read(vaultsProvider).value ?? const [],
                      ref.read(activeVaultProvider),
                    ),
                    StormSpace.agents => Routes.agents,
                  }),
            child: AnimatedContainer(
              duration: t.duration,
              curve: Curves.easeOutCubic,
              alignment: Alignment.center,
              padding: EdgeInsets.symmetric(vertical: t.sp * 0.9),
              decoration: BoxDecoration(
                color: on ? t.accentSoft : Colors.transparent,
                borderRadius: BorderRadius.circular(t.rControl * 0.7),
              ),
              child: Text(
                label,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  fontWeight: on ? FontWeight.w600 : FontWeight.w500,
                  color: on ? t.accent : t.text3,
                ),
              ),
            ),
          ),
        ),
      );
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(t.sp * 1.5, t.sp * 2, t.sp * 1.5, 0),
      child: Container(
        padding: EdgeInsets.all(t.sp * 0.375),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl * 0.8),
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Material(
          type: MaterialType.transparency,
          child: Row(
            children: [
              segment(StormSpace.notes, 'Notes'),
              SizedBox(width: t.sp * 0.375),
              segment(StormSpace.agents, 'Agents'),
            ],
          ),
        ),
      ),
    );
  }
}
