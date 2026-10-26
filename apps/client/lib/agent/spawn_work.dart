import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/controls.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/tokens.dart';
import 'agent_api.dart' show AgentRefusal;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart' show agentCode, agentKey, agentSans;
import 'spawn_api.dart';
import 'spawn_models.dart';
import 'spawn_state.dart';

/// A child's work (137e, 137g; handoff 4o as 138p corrects it): read once
/// it has ended, then Applied into its parent's workspace as a new branch or
/// a new folder the person names, or deleted. Nothing reaches the parent's
/// workspace any other way, and nothing is ever merged or overwritten.

/// What the person is told when the server refuses (137-3's codes).
String childWorkRefusal(String? code, {String? target, required String ws}) =>
    switch (code) {
      'target_exists' =>
        'Something named ${target ?? 'that'} already exists in $ws. Choose '
            'another name; nothing was changed.',
      'branch_exists' =>
        '${target ?? 'Its branch'} already exists in $ws: its work was '
            'applied already, or the branch was made by hand. Nothing was '
            'changed.',
      'child_live' => 'Its work can be applied once it has ended.',
      'no_child' => 'Its workspace is gone: it was deleted, or never made.',
      'isolation_off' => 'Its host can’t reach child workspaces now.',
      'no_review_identity' =>
        'Its host has no review identity yet. Re-run storm-runtime-isolation '
            'on there.',
      'bad_value' => 'Name one new folder: letters, digits, “.”, “_” and “-”.',
      'lineage_mismatch' || 'wrong_host' =>
        'Storm’s record of who started it doesn’t match its host’s. Nothing '
            'was read or changed.',
      _ => 'Couldn’t do it: ${code ?? 'the server didn’t say why'}.',
    };

String _size(int bytes) {
  if (bytes < 1024) return '$bytes B';
  if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
  return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
}

/// The Work tab of a spawned child.
class ChildWorkPanel extends ConsumerStatefulWidget {
  const ChildWorkPanel({super.key, required this.session, this.sheet = false});

  final AgentSession session;
  final bool sheet;

  @override
  ConsumerState<ChildWorkPanel> createState() => _ChildWorkPanelState();
}

class _ChildWorkPanelState extends ConsumerState<ChildWorkPanel> {
  final _name = TextEditingController();
  bool _named = false;
  bool _busy = false;
  bool _confirmDelete = false;
  String? _done;
  String? _refused;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  String _parentWorkspace(AgentOverview? o) {
    final p = widget.session.spawn.parentSession;
    return (p == null ? null : o?.byId(p)?.workspace) ??
        'its parent’s workspace';
  }

  Future<void> _apply(ChildWork w, String ws) async {
    final api = agentApi(ref);
    if (api == null) return;
    final s = widget.session;
    final name = w.clone ? null : _name.text.trim();
    final target = w.clone ? 'storm/${s.id}' : name;
    setState(() {
      _busy = true;
      _refused = null;
    });
    try {
      final answer = await api.applyChildWork(s.id, name: name);
      final commit = shortCommit(answer['commit'] as String?);
      if (!mounted) return;
      setState(
        () => _done = w.clone
            ? 'Applied: the branch storm/${s.id}'
                  '${commit.isEmpty ? '' : ' at $commit'} is in $ws.'
            : 'Applied: the folder $name is in $ws.',
      );
    } catch (e) {
      if (!mounted) return;
      final code = e is AgentRefusal ? e.code : null;
      setState(
        () => _refused = code == null
            ? describeFailure(e)
            : childWorkRefusal(code, target: target, ws: ws),
      );
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _delete(String ws) async {
    final api = agentApi(ref);
    if (api == null) return;
    setState(() => _busy = true);
    final c = ProviderScope.containerOf(context, listen: false);
    try {
      await api.deleteChildWork(widget.session.id);
      c.invalidate(childWorkProvider(widget.session.id));
      if (!mounted) return;
      setState(() {
        _confirmDelete = false;
        _done = 'Deleted: its workspace is gone, and nothing reached $ws.';
      });
    } catch (e) {
      if (!mounted) return;
      final code = e is AgentRefusal ? e.code : null;
      setState(
        () => _refused = code == null
            ? describeFailure(e)
            : childWorkRefusal(code, ws: ws),
      );
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = widget.session;
    final o = ref.watch(agentOverviewProvider).value;
    final ws = _parentWorkspace(o);
    final children = <Widget>[];
    void say(String text, {Key? key, Color? color}) => children.add(
      Text(
        text,
        key: key,
        style: agentSans(t, color: color ?? t.text2),
      ),
    );

    if (!s.ended) {
      say(
        'Nothing reaches $ws until you apply it. Its work can be read and '
        'applied once it has ended.',
        key: const Key('work-live'),
      );
    } else if (_done != null) {
      say(_done!, key: const Key('work-done'), color: t.text);
    } else {
      final work = ref.watch(childWorkProvider(s.id));
      switch (work) {
        case AsyncData(:final value):
          _workView(t, value, ws, children);
        case AsyncError(:final error):
          say(
            childWorkRefusal(error is AgentRefusal ? error.code : null, ws: ws),
            key: const Key('work-refused'),
          );
        default:
          say('Reading its work…');
      }
    }
    if (_refused != null) {
      children.add(
        Text(
          _refused!,
          key: const Key('work-refusal'),
          style: agentSans(t, color: t.danger),
        ),
      );
    }
    if (s.ended && _done == null) {
      children.add(
        _confirmDelete
            ? Container(
                key: const Key('work-delete-confirm'),
                padding: EdgeInsets.all(t.sp * 1.5),
                decoration: BoxDecoration(
                  color: t.surface2,
                  borderRadius: BorderRadius.circular(t.rControl),
                  border: Border.all(color: t.border, width: t.bw),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      'Delete ${s.displayName}’s workspace? Its own copy or '
                      'scratch folder goes, with any of its work you haven’t '
                      'applied. This can’t be undone.',
                      style: agentSans(t),
                    ),
                    SizedBox(height: t.sp),
                    Wrap(
                      alignment: WrapAlignment.end,
                      spacing: t.sp,
                      children: [
                        StormButton.text(
                          label: 'Cancel',
                          onPressed: () =>
                              setState(() => _confirmDelete = false),
                        ),
                        StormButton.danger(
                          key: const Key('work-delete-confirmed'),
                          label: 'Delete workspace',
                          onPressed: _busy ? null : () => _delete(ws),
                        ),
                      ],
                    ),
                  ],
                ),
              )
            : Align(
                alignment: Alignment.centerLeft,
                child: StormButton.text(
                  key: const Key('work-delete'),
                  label: 'Delete its workspace…',
                  onPressed: () => setState(() => _confirmDelete = true),
                ),
              ),
      );
    }

    final column = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (var i = 0; i < children.length; i++) ...[
          if (i > 0) SizedBox(height: t.sp * 1.5),
          children[i],
        ],
      ],
    );
    return widget.sheet
        ? column
        : SingleChildScrollView(
            key: const Key('child-work'),
            padding: EdgeInsets.all(t.sp * 2),
            child: column,
          );
  }

  void _workView(StormTokens t, ChildWork w, String ws, List<Widget> out) {
    final s = widget.session;
    if (!w.clone && !_named) {
      _named = true;
      _name.text = scratchApplyName(ws, s.id);
    }
    out.add(
      Text(
        w.clone
            ? 'Nothing has reached $ws yet. Apply creates the branch '
                  'storm/${s.id} there from its commits. It never merges or '
                  'overwrites.'
            : 'Nothing has reached $ws yet. Apply copies its files into a '
                  'new folder in $ws, named below. It never overwrites: a '
                  'name that exists is refused.',
        key: const Key('work-intro'),
        style: agentSans(t, color: t.text2),
      ),
    );
    if (w.clone) {
      if (w.base != null) {
        out.add(
          Text(
            w.baseFound
                ? 'From ${shortCommit(w.base)}, the commit it started from'
                : 'It rewrote its history, so it is shown from its last '
                      'commit alone',
            key: const Key('work-base'),
            style: agentCode(t),
          ),
        );
      }
      out.add(Text('COMMITS', style: agentKey(t)));
      out.add(
        Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (w.commits.isEmpty)
              Text('No commits: it changed nothing.', style: agentCode(t)),
            for (final c in w.commits)
              Text(
                '${shortCommit(c.id)}  ${c.subject}',
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: agentCode(t, color: t.text),
              ),
            if (w.moreCommits) Text('… and more', style: agentCode(t)),
          ],
        ),
      );
    }
    out.add(Text(w.clone ? 'FILES' : 'ITS FILES', style: agentKey(t)));
    out.add(
      Column(
        key: const Key('work-files'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (w.files.isEmpty) Text('None.', style: agentCode(t)),
          for (final f in w.files)
            Row(
              children: [
                Expanded(
                  child: Text(
                    f.path,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: agentCode(t, color: t.text),
                  ),
                ),
                SizedBox(width: t.sp),
                Text(
                  w.clone
                      ? f.added == null
                            ? 'binary'
                            : '+${f.added} −${f.removed ?? 0}'
                      : _size(f.size ?? 0),
                  style: agentCode(t),
                ),
              ],
            ),
          if (w.moreFiles) Text('… and more', style: agentCode(t)),
          if (w.skipped > 0)
            Text(
              '${w.skipped} links, devices or .git folders are left out.',
              style: agentCode(t),
            ),
        ],
      ),
    );
    if (w.clone && w.patch != null && w.patch!.isNotEmpty) {
      out.add(Text('DIFF', style: agentKey(t)));
      out.add(
        Container(
          key: const Key('work-patch'),
          padding: EdgeInsets.all(t.sp),
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.rControl),
          ),
          child: SelectableText(
            w.patch! +
                (w.patchTruncated ? '\n… the rest is too long to show' : ''),
            style: agentCode(t, color: t.text),
          ),
        ),
      );
    }
    if (!w.clone) {
      out.add(
        TextField(
          key: const Key('work-name'),
          controller: _name,
          enabled: !_busy,
          style: agentCode(t, color: t.text),
          decoration: InputDecoration(labelText: 'New folder in $ws'),
        ),
      );
    }
    out.add(
      Align(
        alignment: Alignment.centerLeft,
        child: StormButton.primary(
          key: const Key('work-apply'),
          label: w.clone ? 'Apply as storm/${s.id}' : 'Apply',
          onPressed: _busy ? null : () => _apply(w, ws),
        ),
      ),
    );
  }
}
