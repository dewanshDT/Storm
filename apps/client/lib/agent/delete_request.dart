import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/controls.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/tokens.dart';
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart' show agentKey, agentMono, agentSans;

/// The agent asking, as the dialog names it.
class DeleteAsker {
  const DeleteAsker({
    required this.sessionId,
    required this.name,
    required this.provider,
    this.model,
    this.context,
  });

  final String sessionId;
  final String name;
  final String provider;
  final String? model;
  final SessionContext? context;

  factory DeleteAsker.fromJson(Map<String, dynamic> j) => DeleteAsker(
    sessionId: j['id'] as String? ?? '',
    name: (j['title'] ?? j['name'] ?? 'An agent') as String,
    provider: j['provider'] as String? ?? '',
    model: j['model'] as String?,
    context: j['context'] is Map<String, dynamic>
        ? SessionContext.fromJson(j['context'] as Map<String, dynamic>)
        : null,
  );

  factory DeleteAsker.of(AgentSession s) => DeleteAsker(
    sessionId: s.id,
    name: s.displayName,
    provider: s.provider,
    model: s.model,
    context: s.context,
  );
}

/// The routes of decision 149, owner only on the session tier.
extension DeleteApi on AgentApi {
  /// The deletions waiting, and those just answered; with [after], waits up
  /// to [wait] seconds for a change first.
  Future<(int, List<(DeleteRequest, DeleteAsker)>)> deleteRequests({
    int? after,
    int wait = 0,
  }) async {
    final json =
        await send(
              'GET',
              '/v1/agent/delete-requests',
              query: {
                if (after != null) 'after': '$after',
                if (after != null) 'wait': '$wait',
              },
            )
            as Map;
    return (
      (json['version'] as num?)?.toInt() ?? 0,
      [
        for (final r in json['requests'] as List? ?? const [])
          if (r is Map<String, dynamic>)
            (
              DeleteRequest.fromJson(r),
              DeleteAsker.fromJson(
                r['session'] is Map<String, dynamic>
                    ? r['session'] as Map<String, dynamic>
                    : const {},
              ),
            ),
      ],
    );
  }

  Future<DeleteRequest> decideDeleteRequest(
    String sessionId,
    String requestId, {
    required bool approve,
  }) async => DeleteRequest.fromJson(
    await send(
          'POST',
          '/v1/agent/sessions/${Uri.encodeComponent(sessionId)}'
              '/delete-requests/${Uri.encodeComponent(requestId)}',
          body: {'decision': approve ? 'approve' : 'decline'},
        )
        as Map<String, dynamic>,
  );
}

/// "Delete *Ideas.md*?" or "Delete folder *Projects/Old* and its 12 notes?".
/// The emphasised part is the target.
(String, String, String) deleteTitle(DeleteRequest r) {
  if (!r.isFolder) {
    return ('Delete ', r.path.split('/').last, '?');
  }
  final rest = switch (r.notes) {
    0 => '?',
    1 => ' and its 1 note?',
    final n => ' and its $n notes?',
  };
  return ('Delete folder ', r.path, rest);
}

const kDeleteFootnote = 'It goes to Trash, which keeps it for 30 days.';

const kDeleteLaterNote =
    'Later closes this and keeps the request waiting: Review it from the '
    'session until it times out.';

String _bytes(int n) {
  if (n < 1024) return '$n B';
  if (n < 1024 * 1024) return '${(n / 1024).toStringAsFixed(1)} KB';
  return '${(n / (1024 * 1024)).toStringAsFixed(1)} MB';
}

String _when(String? iso) {
  final at = DateTime.tryParse(iso ?? '')?.toLocal();
  if (at == null) return '';
  String two(int v) => v.toString().padLeft(2, '0');
  return '${at.year}-${two(at.month)}-${two(at.day)} '
      '${two(at.hour)}:${two(at.minute)}';
}

/// What a dialog ended with: the person's answer, or another device's.
enum _Ended { approve, decline, elsewhere }

/// The dialogs open on this device, by request, each with how to close it
/// when another device answers first.
final _open = <String, void Function()>{};

/// Asks the person (149c, amended 2026-10-31). Approve is the main,
/// destructive button and only Decline declines; Later, Esc and dismissing
/// it close it on this device and leave the request pending. Nothing when
/// its dialog is already open here.
Future<void> showDeleteRequestDialog(
  BuildContext context,
  ProviderContainer container,
  DeleteRequest r,
  DeleteAsker asker,
) async {
  if (_open.containsKey(r.id)) return;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final ended = await showDialog<_Ended>(
    context: context,
    useRootNavigator: true,
    builder: (dialog) {
      void close() {
        if (dialog.mounted) Navigator.of(dialog).pop(_Ended.elsewhere);
      }

      _open[r.id] = close;
      return DeleteRequestDialog(
        request: r,
        asker: asker,
        onGone: () {
          if (_open[r.id] == close) _open.remove(r.id);
        },
        onOpenSession: () {
          Navigator.of(dialog).pop(_Ended.elsewhere);
          _open.remove(r.id);
          if (context.mounted) openAgentSession(context, asker.sessionId);
        },
      );
    },
  );
  _open.remove(r.id);
  if (ended == null || ended == _Ended.elsewhere) return;
  final api = container.read(agentApiFactoryProvider)?.call();
  if (api == null) return;
  final approve = ended == _Ended.approve;
  try {
    await api.decideDeleteRequest(r.sessionId, r.id, approve: approve);
  } catch (e) {
    final code = e is AgentRefusal ? e.code : null;
    messenger?.showSnackBar(
      SnackBar(
        content: Text(switch (code) {
          'request_timed_out' => 'That request timed out: nothing was deleted.',
          final c? when c.startsWith('request_') => 'It was already answered.',
          _ => describeFailure(e),
        }),
      ),
    );
  } finally {
    api.dispose();
    container.invalidate(sessionDetailProvider(r.sessionId));
  }
}

/// Closes this device's dialog for [r], answered elsewhere, and says who.
void closeAnsweredElsewhere(BuildContext context, DeleteRequest r) {
  final close = _open.remove(r.id);
  if (close == null) return;
  close();
  final who = r.decidedBy;
  ScaffoldMessenger.maybeOf(context)?.showSnackBar(
    SnackBar(
      key: const Key('delete-answered-elsewhere'),
      content: Text(switch (r.state) {
        'timed_out' => 'The request to delete ${r.path} timed out.',
        _ => 'Answered on ${who ?? 'another device'}',
      }),
    ),
  );
}

class DeleteRequestDialog extends StatefulWidget {
  const DeleteRequestDialog({
    super.key,
    required this.request,
    required this.asker,
    required this.onOpenSession,
    this.onGone,
  });

  final DeleteRequest request;
  final DeleteAsker asker;
  final VoidCallback onOpenSession;

  /// Called once it is off screen, however it closed.
  final VoidCallback? onGone;

  @override
  State<DeleteRequestDialog> createState() => _DeleteRequestDialogState();
}

class _DeleteRequestDialogState extends State<DeleteRequestDialog> {
  bool _all = false;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_key);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_key);
    widget.onGone?.call();
    super.dispose();
  }

  /// Esc closes it, as Later does, wherever focus is: a page the shell's
  /// own navigator finishes while this is open takes focus.
  bool _key(KeyEvent e) {
    if (e is! KeyDownEvent || e.logicalKey != LogicalKeyboardKey.escape) {
      return false;
    }
    final route = ModalRoute.of(context);
    if (!mounted || route == null || !route.isCurrent) return false;
    Navigator.of(context).pop();
    return true;
  }

  @override
  Widget build(BuildContext context) => _dialog(context);

  Widget _dialog(BuildContext context) {
    final t = context.tokens;
    final r = widget.request;
    final a = widget.asker;
    final (lead, target, tail) = deleteTitle(r);
    final agent = [
      providerLabel(a.provider),
      if (a.model != null) a.model!,
    ].join(' · ');
    const shown = 5;
    final paths = _all ? r.paths : r.paths.take(shown).toList();
    final hidden = r.notes - paths.length;
    return AlertDialog(
      key: const Key('delete-request-dialog'),
      title: Text.rich(
        key: const Key('delete-request-title'),
        TextSpan(
          children: [
            TextSpan(text: lead),
            TextSpan(
              text: target,
              style: const TextStyle(fontStyle: FontStyle.italic),
            ),
            TextSpan(text: tail),
          ],
        ),
      ),
      content: SizedBox(
        width: 480,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text('WHO', style: agentKey(t)),
              SizedBox(height: t.sp * 0.5),
              Text(
                a.name,
                key: const Key('delete-request-agent'),
                style: agentSans(t, weight: FontWeight.w600),
              ),
              Text(agent, style: agentSans(t, color: t.text2)),
              if (a.context case final note?)
                Text(
                  'Started from ${note.title}',
                  key: const Key('delete-request-context'),
                  style: agentSans(t, color: t.text2),
                ),
              Align(
                alignment: Alignment.centerLeft,
                child: StormButton.text(
                  key: const Key('delete-request-open-session'),
                  label: 'Open session',
                  onPressed: widget.onOpenSession,
                ),
              ),
              SizedBox(height: t.sp),
              Text('${a.name.toUpperCase()} SAYS:', style: agentKey(t)),
              SizedBox(height: t.sp * 0.5),
              Container(
                padding: EdgeInsets.all(t.sp),
                decoration: BoxDecoration(
                  color: t.surface2,
                  borderRadius: BorderRadius.circular(t.rControl),
                ),
                // The agent's words: plain text, never markup or a link.
                child: Text(
                  r.reason,
                  key: const Key('delete-request-reason'),
                  style: agentMono(t, color: t.text2),
                ),
              ),
              SizedBox(height: t.sp * 1.5),
              Text('WHAT', style: agentKey(t)),
              SizedBox(height: t.sp * 0.5),
              if (!r.isFolder)
                Text(
                  [
                    r.path,
                    _bytes(r.size),
                    if (_when(r.modified).isNotEmpty)
                      'modified ${_when(r.modified)}',
                  ].join(' · '),
                  key: const Key('delete-request-what'),
                  style: agentMono(t),
                )
              else ...[
                Text(
                  r.notes == 1 ? '1 note' : '${r.notes} notes',
                  key: const Key('delete-request-count'),
                  style: agentSans(t),
                ),
                for (final p in paths)
                  Text(
                    p,
                    key: ValueKey('delete-request-path-$p'),
                    style: agentMono(t, color: t.text2),
                  ),
                if (hidden > 0)
                  Align(
                    alignment: Alignment.centerLeft,
                    child: StormButton.text(
                      key: const Key('delete-request-more'),
                      label: _all || r.paths.length <= shown
                          ? '…and $hidden more'
                          : 'Show $hidden more',
                      onPressed: _all || r.paths.length <= shown
                          ? null
                          : () => setState(() => _all = true),
                    ),
                  ),
              ],
              SizedBox(height: t.sp * 1.5),
              Text(
                kDeleteFootnote,
                key: const Key('delete-request-footnote'),
                style: agentSans(t, color: t.text2),
              ),
              SizedBox(height: t.sp * 0.5),
              Text(
                kDeleteLaterNote,
                key: const Key('delete-request-later-note'),
                style: agentSans(t, color: t.text2),
              ),
            ],
          ),
        ),
      ),
      actions: [
        StormButton.text(
          key: const Key('delete-request-later'),
          label: 'Later',
          onPressed: () => Navigator.of(context).pop(),
        ),
        StormButton.outline(
          key: const Key('delete-request-decline'),
          label: 'Decline',
          onPressed: () => Navigator.of(context).pop(_Ended.decline),
        ),
        StormButton.danger(
          key: const Key('delete-request-approve'),
          label: 'Approve',
          onPressed: () => Navigator.of(context).pop(_Ended.approve),
        ),
      ],
    );
  }
}

/// Watches for agents' deletions on every screen while signed in (149c):
/// each new one opens its dialog, and one answered on another device
/// closes here with who answered. A long poll, paused in the background,
/// and stopped on a server without the route.
class DeleteRequestWatcher extends ConsumerStatefulWidget {
  const DeleteRequestWatcher({super.key, required this.child});

  final Widget child;

  @override
  ConsumerState<DeleteRequestWatcher> createState() =>
      _DeleteRequestWatcherState();
}

/// How long one poll waits for a change at the server.
const kDeleteWatchWait = 20;

class _DeleteRequestWatcherState extends ConsumerState<DeleteRequestWatcher> {
  int? _version;
  bool _running = false;
  bool _stopped = false;
  Timer? _retry;
  AppLifecycleListener? _lifecycle;

  /// Requests this device already showed, so one put off with Later is
  /// not opened again while it is still listed: Review reopens it.
  final _seen = <String>{};

  @override
  void initState() {
    super.initState();
    _lifecycle = AppLifecycleListener(onResume: _poll);
    WidgetsBinding.instance.addPostFrameCallback((_) => _poll());
  }

  @override
  void dispose() {
    _stopped = true;
    _retry?.cancel();
    _lifecycle?.dispose();
    super.dispose();
  }

  bool get _foreground {
    final s = WidgetsBinding.instance.lifecycleState;
    return s == null || s == AppLifecycleState.resumed;
  }

  Future<void> _poll() async {
    if (_running || _stopped || !mounted || !_foreground) return;
    final api = ref.read(agentApiFactoryProvider)?.call();
    if (api == null) return;
    _running = true;
    _retry?.cancel();
    var again = Duration.zero;
    try {
      final (version, requests) = await api.deleteRequests(
        after: _version,
        wait: kDeleteWatchWait,
      );
      if (!mounted || _stopped) return;
      _version = version;
      _show(requests);
    } on AgentRefusal catch (e) {
      // A server before decision 149 has no such route: nothing to watch.
      if (e.statusCode == 404 || e.statusCode == 403) {
        _stopped = true;
        return;
      }
      again = const Duration(seconds: 5);
    } catch (_) {
      again = const Duration(seconds: 5);
    } finally {
      api.dispose();
      _running = false;
    }
    if (!mounted || _stopped) return;
    // A floor between polls, so a server that answers at once is never
    // asked in a tight loop.
    _retry = Timer(
      again == Duration.zero ? const Duration(seconds: 1) : again,
      _poll,
    );
  }

  void _show(List<(DeleteRequest, DeleteAsker)> requests) {
    final at = context;
    for (final (r, _) in requests) {
      if (!r.pending) closeAnsweredElsewhere(at, r);
    }
    final next = requests
        .where((e) => e.$1.pending && !_seen.contains(e.$1.id))
        .firstOrNull;
    if (next == null) return;
    _seen.add(next.$1.id);
    final container = ProviderScope.containerOf(context, listen: false);
    unawaited(showDeleteRequestDialog(at, container, next.$1, next.$2));
  }

  @override
  Widget build(BuildContext context) {
    ref.listen(agentApiFactoryProvider, (_, next) {
      _version = null;
      if (next != null) _poll();
    });
    return widget.child;
  }
}

/// A session's Needs you entry for its agent's deletion (149c), so it is
/// not lost when the dialog was closed, here or elsewhere: Review opens it.
class DeleteRequestCard extends ConsumerWidget {
  const DeleteRequestCard({super.key, required this.session});

  final AgentSession session;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final r = session.deleteRequest;
    if (r == null || !r.pending) return const SizedBox.shrink();
    final t = context.tokens;
    final (lead, target, tail) = deleteTitle(r);
    return Semantics(
      container: true,
      liveRegion: true,
      child: Container(
        key: const Key('delete-request-card'),
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 2,
          vertical: t.sp * 1.5,
        ),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl),
          border: Border.all(color: t.amber, width: t.bw),
        ),
        child: Row(
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('NEEDS YOU', style: agentKey(t)),
                  SizedBox(height: t.sp * 0.5),
                  Text(
                    '${session.displayName} asks: $lead$target$tail',
                    style: agentSans(t, weight: FontWeight.w600),
                  ),
                ],
              ),
            ),
            SizedBox(width: t.sp),
            StormButton.primary(
              key: const Key('delete-request-review'),
              label: 'Review',
              onPressed: () => showDeleteRequestDialog(
                context,
                ProviderScope.containerOf(context, listen: false),
                r,
                DeleteAsker.of(session),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
