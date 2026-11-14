import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../agent/agent_notices.dart';
import '../../agent/delete_request.dart';
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/frecency.dart';
import '../../state/nav_memory.dart';
import '../../keyboard/storm_app_shortcuts.dart';
import '../breakpoints.dart';
import 'activity_rail.dart';
import 'app_bar.dart';
import 'create_here.dart' show pageArrivalProvider;
import 'nav_history.dart';
import 'sidebar_frame.dart';
import 'window_buttons.dart';

/// Where system back goes once nothing is left to pop, or null to leave the
/// app (decision Q2): a note goes to its folder, a folder to its parent,
/// Agents and Settings to the last Notes location; the vault root exits.
String? logicalParent(
  String location, {
  required bool wide,
  required NavMemory memory,
  String? Function(String noteId)? folderOfNote,
}) {
  final uri = Uri.parse(location);
  final path = uri.path;
  final seg = uri.pathSegments;
  final notes = memory.entryOf(Activity.notes);

  if (seg.length >= 3 && seg[0] == 'v') {
    final vault = seg[1];
    switch (seg[2]) {
      case 'note':
        final folder = seg.length > 3 ? folderOfNote?.call(seg[3]) : null;
        return Routes.folder(vault, folder ?? '');
      case 'browse':
        final folder = Routes.folderOf(uri);
        if (folder.isEmpty) return null;
        final cut = folder.lastIndexOf('/');
        return Routes.folder(vault, cut < 0 ? '' : folder.substring(0, cut));
      default:
        return Routes.browse(vault);
    }
  }
  if (path.startsWith('/agents/')) return Routes.agents;
  if (path.startsWith('/settings/') && !wide) return Routes.settings;
  if (activityOf(path) case Activity.agents || Activity.settings) return notes;
  return null;
}

String _folderOf(String notePath) {
  final cut = notePath.lastIndexOf('/');
  return cut < 0 ? '' : notePath.substring(0, cut);
}

String? _parentOf(WidgetRef ref, BuildContext context, String location) {
  final tree = ref.read(treeProvider).value ?? const [];
  return logicalParent(
    location,
    wide: context.isExpanded,
    memory: ref.read(navMemoryProvider),
    folderOfNote: (id) {
      final meta = tree.where((n) => n.id == id).firstOrNull;
      return meta == null ? null : _folderOf(meta.path);
    },
  );
}

/// Claims system back for a page that has a logical parent, so the platform
/// asks the app instead of closing it. Android 16's predictive back (target
/// SDK 36) only asks when a route can pop or blocks the pop; a page reached
/// with `go` can do neither.
class LogicalBack extends ConsumerWidget {
  const LogicalBack({super.key, required this.child, this.shell = false});

  final Widget child;

  /// A shell's page is never pushed; the pages inside it may be.
  final bool shell;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pushed = !shell && !(ModalRoute.of(context)?.isFirst ?? true);
    ref.watch(treeProvider);
    final parent = _parentOf(
      ref,
      context,
      GoRouterState.of(context).uri.toString(),
    );
    return PopScope(
      canPop: pushed || parent == null,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && parent != null) context.go(parent);
      },
      child: child,
    );
  }
}

/// Every signed-in location: the app bar over the rail and the pane at desk
/// width (decision 153), the phone's corner bubbles (drawn by each screen's
/// chrome) below it.
class AppShell extends ConsumerStatefulWidget {
  const AppShell({super.key, required this.location, required this.child});

  final String location;
  final Widget child;

  @override
  ConsumerState<AppShell> createState() => _AppShellState();
}

class _AppShellState extends ConsumerState<AppShell> {
  @override
  void initState() {
    super.initState();
    _remember();
  }

  @override
  void didUpdateWidget(AppShell old) {
    super.didUpdateWidget(old);
    if (old.location != widget.location) _remember();
  }

  void _remember() {
    final location = widget.location;
    ref.read(pageArrivalProvider).visit(Uri.parse(location));
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      ref.read(navMemoryProvider.notifier).visit(location);
      ref.read(navHistoryProvider.notifier).visit(location);
      if (frecencyIdOf(location) case final id?) {
        ref.read(frecencyProvider.notifier).opened(id);
      }
    });
  }

  Future<bool> _back() async {
    final router = GoRouter.of(context);
    if (await router.routerDelegate.popRoute()) return true;
    if (!mounted) return true;
    final parent = _parentOf(ref, context, router.state.uri.toString());
    if (parent == null) return false;
    router.go(parent);
    return true;
  }

  @override
  Widget build(BuildContext context) {
    final child = BackButtonListener(
      onBackButtonPressed: _back,
      child: widget.child,
    );
    final wide = context.isExpanded;
    final Widget body;
    if (wide) {
      body = Scaffold(
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const StormAppBar(),
            Expanded(
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  ActivityRail(
                    current: activityOf(Uri.parse(widget.location).path),
                  ),
                  Expanded(child: PaneSemantics(child: child)),
                ],
              ),
            ),
          ],
        ),
      );
    } else if (windowChrome != WindowChrome.none) {
      // A narrow desktop window keeps the phone layout, and still needs a
      // frame to move and close it by.
      body = WindowFrame(child: child);
    } else {
      body = child;
    }
    // Outermost on both sides of the breakpoint, so crossing it keeps the
    // watches (decisions 149c, 154).
    return DeleteRequestWatcher(
      location: widget.location,
      child: AgentNoticeWatcher(
        location: widget.location,
        child: StormAppShortcuts(child: body),
      ),
    );
  }
}
