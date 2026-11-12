/// The pieces of Settings › Agents & hosts (153-7): the grid, a host's
/// details under it, a cell's popover, the phone's list and the empty card.
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../state/health.dart' show relativeTime;
import '../ui/breakpoints.dart';
import '../ui/clipboard_copy.dart';
import '../ui/controls.dart';
import '../ui/settings/settings_widgets.dart';
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agents_settings_screen.dart';

/// "Runs as dewansh": who the host runs as, always shown (153-7 copy rule).
String runsAsShort(AgentHost h) {
  final account = h.execution?.account;
  if (account == null || account.isEmpty) return kRunsAsUnknown;
  return 'Runs as $account';
}

/// The host cell's lines under its name: its state when it is not online,
/// then who it runs as. Never trusted, secure, protected or sandboxed.
List<String> hostLines(AgentHost h) {
  final seen = DateTime.tryParse(h.lastSeen ?? '');
  return [
    if (h.status == 'revoked')
      'Revoked'
    else if (!h.online)
      seen == null ? 'Offline' : 'Offline · last seen ${relativeTime(seen)}',
    runsAsShort(h),
  ];
}

/// The neutral dot: filled `text2` online, a `text3` ring otherwise. Never
/// green.
class HostDot extends StatelessWidget {
  const HostDot({super.key, required this.online});

  final bool online;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      width: t.sp,
      height: t.sp,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: online ? t.text2 : null,
        border: online ? null : Border.all(color: t.text3, width: t.bw * 1.5),
      ),
    );
  }
}

/// The dot, the name at 14/500 and the lines under it.
class HostLabel extends StatelessWidget {
  const HostLabel({super.key, required this.host, this.keyed = true});

  final AgentHost host;

  /// False for the copy the pinned column measures with.
  final bool keyed;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final h = host;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      spacing: t.sp * 1.25,
      children: [
        Padding(
          padding: EdgeInsets.only(top: t.sp * 0.75),
          child: HostDot(
            key: keyed ? Key('host-dot-${h.id}') : null,
            online: h.online,
          ),
        ),
        Flexible(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            spacing: t.sp * 0.375,
            children: [
              Text(
                h.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.uiSize,
                  fontWeight: FontWeight.w500,
                  color: h.online ? t.text : t.text2,
                ),
              ),
              Text(
                hostLines(h).join('\n'),
                key: keyed ? Key('host-sub-${h.id}') : null,
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: t.text3,
                  height: 1.4,
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

typedef OpenCell = ({String host, String agent});

/// Hosts as rows, agents as columns (153-7). At a width where five agent
/// columns no longer fit at their type, the host column stays put and the
/// agents scroll sideways (153n).
class AgentsGrid extends StatefulWidget {
  const AgentsGrid({
    super.key,
    required this.hosts,
    required this.defaultProvider,
    required this.runs,
    required this.selected,
    required this.open,
    required this.hostAdmin,
    required this.link,
    required this.onSelect,
    required this.onOpen,
    required this.onMakeDefault,
    required this.onReenroll,
    required this.onRemove,
  });

  final List<AgentHost> hosts;
  final String? defaultProvider;
  final Map<String, AgentRun> runs;
  final String? selected;
  final OpenCell? open;

  /// The server can remove and re-enroll a revoked host.
  final bool hostAdmin;
  final LayerLink Function(String host, String agent) link;
  final ValueChanged<AgentHost> onSelect;
  final void Function(AgentHost host, String agent) onOpen;
  final ValueChanged<String> onMakeDefault;
  final ValueChanged<AgentHost> onReenroll;
  final ValueChanged<AgentHost> onRemove;

  @override
  State<AgentsGrid> createState() => _AgentsGridState();
}

class _AgentsGridState extends State<AgentsGrid> {
  final _scroll = ScrollController();

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final hostWidth = t.sp * 21.25;
    // The narrowest an agent column gets before it would need smaller type:
    // "needs Node 22" in mono, or "Command Code" at the header's size.
    final minColumn = t.sp * 12.5;
    return LayoutBuilder(
      builder: (context, c) {
        final column = math.max(minColumn, (c.maxWidth - hostWidth - 2) / 5);
        return Container(
          key: const Key('agents-grid'),
          clipBehavior: Clip.antiAlias,
          decoration: BoxDecoration(
            color: t.surface,
            borderRadius: BorderRadius.circular(t.rCard),
            border: Border.all(color: t.border, width: t.bw),
          ),
          child: Scrollbar(
            controller: _scroll,
            child: SingleChildScrollView(
              key: const Key('agents-grid-scroll'),
              controller: _scroll,
              scrollDirection: Axis.horizontal,
              child: SizedBox(
                width: hostWidth + column * 5,
                child: ListenableBuilder(
                  listenable: _scroll,
                  builder: (context, _) {
                    final pin = _scroll.hasClients ? _scroll.offset : 0.0;
                    return Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        _header(t, hostWidth, column, pin),
                        for (final h in widget.hosts)
                          _row(t, h, hostWidth, column, pin),
                      ],
                    );
                  },
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  /// A row whose first [hostWidth] stays where it is while the rest
  /// scrolls: the host cell is drawn over the agents at the scroll offset,
  /// and an unseen copy in the row gives the row its height.
  Widget _pinned({
    Key? key,
    required StormTokens t,
    required double hostWidth,
    required double pin,
    required Color background,
    required Widget host,
    required Widget measure,
    required List<Widget> rest,
    bool top = true,
  }) => Container(
    key: key,
    decoration: BoxDecoration(
      color: background,
      border: top
          ? Border(
              top: BorderSide(color: t.border, width: t.bw),
            )
          : null,
    ),
    child: Stack(
      children: [
        IntrinsicHeight(
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SizedBox(
                width: hostWidth,
                child: ExcludeSemantics(
                  child: Visibility(
                    visible: false,
                    maintainSize: true,
                    maintainAnimation: true,
                    maintainState: true,
                    child: measure,
                  ),
                ),
              ),
              ...rest,
            ],
          ),
        ),
        Positioned(
          left: pin,
          top: 0,
          bottom: 0,
          width: hostWidth,
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: background,
              // Once the agents scroll under it, an edge says where.
              border: pin > 0
                  ? Border(
                      right: BorderSide(color: t.border, width: t.bw),
                    )
                  : null,
            ),
            child: host,
          ),
        ),
      ],
    ),
  );

  Widget _header(StormTokens t, double hostWidth, double column, double pin) {
    Widget hostHead() => Padding(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 2, vertical: t.sp * 1.5),
      child: Text(
        'HOST',
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.labelSize,
          letterSpacing: t.labelSize * 0.08,
          color: t.text3,
        ),
      ),
    );
    return _pinned(
      t: t,
      hostWidth: hostWidth,
      pin: pin,
      top: false,
      background: t.surface,
      host: Align(alignment: Alignment.centerLeft, child: hostHead()),
      measure: hostHead(),
      rest: [
        for (final a in kInstallableAgents)
          SizedBox(
            width: column,
            child: _AgentHead(
              agent: a,
              isDefault: a == widget.defaultProvider,
              onTap: () => widget.onMakeDefault(a),
            ),
          ),
      ],
    );
  }

  Widget _row(
    StormTokens t,
    AgentHost h,
    double hostWidth,
    double column,
    double pin,
  ) {
    final revoked = h.status == 'revoked';
    final selected = h.id == widget.selected;
    final background = selected ? t.surface2 : t.surface;
    Widget hostCell({required bool keyed}) {
      final label = Padding(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 2,
          vertical: t.sp * 1.5,
        ),
        child: HostLabel(host: h, keyed: keyed),
      );
      if (!keyed) return label;
      return Opacity(
        opacity: revoked ? 0.75 : 1,
        child: InkWell(
          key: Key('host-cell-${h.id}'),
          onTap: revoked ? null : () => widget.onSelect(h),
          child: Align(alignment: Alignment.topLeft, child: label),
        ),
      );
    }

    return _pinned(
      key: Key('host-row-${h.id}'),
      t: t,
      hostWidth: hostWidth,
      pin: pin,
      background: background,
      host: hostCell(keyed: true),
      measure: hostCell(keyed: false),
      rest: revoked
          ? [
              SizedBox(
                width: column * 5,
                child: Padding(
                  padding: EdgeInsets.symmetric(horizontal: t.sp * 2),
                  child: RevokedActions(
                    host: h,
                    hostAdmin: widget.hostAdmin,
                    onReenroll: () => widget.onReenroll(h),
                    onRemove: () => widget.onRemove(h),
                  ),
                ),
              ),
            ]
          : [
              for (final a in kInstallableAgents)
                SizedBox(
                  width: column,
                  child: CompositedTransformTarget(
                    link: widget.link(h.id, a),
                    child: AgentCellView(
                      host: h,
                      agent: a,
                      cell: agentCell(h, a, widget.runs['${h.id}/$a']),
                      open:
                          widget.open?.host == h.id && widget.open?.agent == a,
                      onTap: () => widget.onOpen(h, a),
                    ),
                  ),
                ),
            ],
    );
  }
}

/// An agent's column header: its name, and `default` on the default one.
/// Clicking another makes it the default.
class _AgentHead extends StatelessWidget {
  const _AgentHead({
    required this.agent,
    required this.isDefault,
    required this.onTap,
  });

  final String agent;
  final bool isDefault;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final name = providerLabel(agent);
    return Tooltip(
      message: isDefault ? 'The default agent' : 'Make $name the default',
      child: InkWell(
        key: Key('agent-head-$agent'),
        onTap: isDefault ? null : onTap,
        child: Padding(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 0.25,
            vertical: t.sp * 1.25,
          ),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            spacing: t.sp * 0.375,
            children: [
              Text(
                name,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  fontWeight: FontWeight.w500,
                  color: t.text,
                  height: 1.25,
                ),
              ),
              if (isDefault) DefaultTag(key: Key('default-tag-$agent')),
            ],
          ),
        ),
      ),
    );
  }
}

/// `default`, mono, in `accent`.
class DefaultTag extends StatelessWidget {
  const DefaultTag({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 0.625),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(t.rControl * 0.5),
        border: Border.all(color: t.accent, width: t.bw),
      ),
      child: Text(
        'default',
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.labelSize,
          color: t.accent,
          height: 1.3,
        ),
      ),
    );
  }
}

/// The colour of a cell's word: `accent` only where you can act. On an
/// open cell's `accentSoft` a quiet word is `text2`: `text3` misses 4.5:1
/// there in dark and earth (153-8).
Color cellWordColor(StormTokens t, AgentCell cell, {bool open = false}) =>
    cell.actionable
    ? t.accent
    : cell.word == CellWord.ready
    ? t.text
    : (open ? t.text2 : t.text3);

/// One cell: the word, then the version, step or reason under it.
class AgentCellView extends StatelessWidget {
  const AgentCellView({
    super.key,
    required this.host,
    required this.agent,
    required this.cell,
    required this.open,
    required this.onTap,
  });

  final AgentHost host;
  final String agent;
  final AgentCell cell;
  final bool open;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final id = '${host.id}-$agent';
    return Semantics(
      button: true,
      label: '${providerLabel(agent)} on ${host.name}: ${cell.top}',
      child: Material(
        color: open ? t.accentSoft : Colors.transparent,
        child: InkWell(
          key: Key('cell-$id'),
          onTap: onTap,
          child: Container(
            decoration: BoxDecoration(
              border: open
                  ? Border.all(color: t.accent, width: t.bw * 1.5)
                  : Border(
                      left: BorderSide(color: t.border, width: t.bw),
                    ),
            ),
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 0.25,
              vertical: t.sp * 1.25,
            ),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              spacing: t.sp * 0.5,
              children: [
                Text(
                  cell.top,
                  key: Key('cell-word-$id'),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    fontWeight: cell.actionable
                        ? FontWeight.w500
                        : FontWeight.w400,
                    color: cellWordColor(t, cell, open: open),
                  ),
                ),
                if (cell.progress != null)
                  ProgressBar(key: Key('cell-bar-$id'), value: cell.progress!),
                if (cell.sub.isNotEmpty)
                  // Mono 10.5 in the frames; 11 here, the smallest step
                  // the tokens allow (`minStep`).
                  Text(
                    cell.sub,
                    key: Key('cell-sub-$id'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: open ? t.text2 : t.text3,
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

/// The 3px bar under "Installing".
class ProgressBar extends StatelessWidget {
  const ProgressBar({super.key, required this.value});

  final double value;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final width = t.sp * 7;
    return ClipRRect(
      borderRadius: BorderRadius.circular(t.sp * 0.25),
      child: Container(
        width: width,
        height: t.sp * 0.375,
        color: t.surface2,
        alignment: Alignment.centerLeft,
        child: Container(width: width * value, color: t.text3),
      ),
    );
  }
}

/// A revoked host's line and its two actions; Remove confirms in place.
class RevokedActions extends StatefulWidget {
  const RevokedActions({
    super.key,
    required this.host,
    required this.hostAdmin,
    required this.onReenroll,
    required this.onRemove,
  });

  final AgentHost host;
  final bool hostAdmin;
  final VoidCallback onReenroll;
  final VoidCallback onRemove;

  @override
  State<RevokedActions> createState() => _RevokedActionsState();
}

class _RevokedActionsState extends State<RevokedActions> {
  bool _confirming = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text3,
    );
    final id = widget.host.id;
    if (_confirming && widget.hostAdmin) {
      return Row(
        key: Key('remove-strip-$id'),
        spacing: t.sp * 1.75,
        children: [
          Expanded(
            child: Text(
              'Remove ${widget.host.name} from the list? Its ended sessions '
              'keep its name.',
              style: style.copyWith(color: t.text2),
            ),
          ),
          TextAction(
            key: const Key('remove-host-confirm'),
            label: 'Remove',
            danger: true,
            onTap: () {
              setState(() => _confirming = false);
              widget.onRemove();
            },
          ),
          TextAction(
            key: const Key('remove-host-cancel'),
            label: 'Cancel',
            onTap: () => setState(() => _confirming = false),
          ),
        ],
      );
    }
    return Row(
      spacing: t.sp * 1.75,
      children: [
        Expanded(
          child: Opacity(
            opacity: 0.75,
            child: Text("It can't connect again.", style: style),
          ),
        ),
        if (widget.hostAdmin) ...[
          TextAction(
            key: Key('reenroll-host-$id'),
            label: 'Re-enroll…',
            accent: true,
            onTap: widget.onReenroll,
          ),
          TextAction(
            key: Key('remove-host-$id'),
            label: 'Remove',
            danger: true,
            onTap: () => setState(() => _confirming = true),
          ),
        ],
      ],
    );
  }
}

/// The selected host under the grid: its name and state, Rename and
/// Revoke…, who it runs as, and its Limits as steppers (decision 152).
class HostDetails extends StatefulWidget {
  const HostDetails({
    super.key,
    required this.host,
    required this.onRename,
    required this.onRevoke,
    required this.onSaveLimits,
  });

  final AgentHost host;
  final Future<void> Function(String name) onRename;
  final Future<void> Function() onRevoke;

  /// Answers why it was refused, or null once saved.
  final Future<String?> Function(int children, int tree, int? sessions)
  onSaveLimits;

  @override
  State<HostDetails> createState() => _HostDetailsState();
}

class _HostDetailsState extends State<HostDetails> {
  bool _renaming = false;
  bool _confirmRevoke = false;
  late final _name = TextEditingController(text: widget.host.name);

  late HostLimits _limits = widget.host.limits ?? const HostLimits();
  int? _sessionsOverride;
  bool _saving = false;
  String? _limitsError;

  @override
  void initState() {
    super.initState();
    _sessionsOverride = _limits.maxSessions;
  }

  @override
  void didUpdateWidget(HostDetails old) {
    super.didUpdateWidget(old);
    final next = widget.host.limits;
    if (!_saving && next != null && next != old.host.limits) {
      _limits = next;
      _sessionsOverride = next.maxSessions;
    }
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _save({
    int? children,
    int? tree,
    int? sessions,
    bool clear = false,
  }) async {
    final c = children ?? _limits.maxChildren;
    final tr = tree ?? _limits.maxTree;
    final s = clear ? null : (sessions ?? _sessionsOverride);
    if (tr < c) {
      setState(
        () => _limitsError =
            'Agents per tree must be at least agents per session.',
      );
      return;
    }
    final was = (_limits, _sessionsOverride);
    setState(() {
      _saving = true;
      _limitsError = null;
      _limits = HostLimits(
        maxChildren: c,
        maxTree: tr,
        maxSessions: s,
        hostMaxSessions: _limits.hostMaxSessions,
        effectiveMaxSessions: _limits.effectiveMaxSessions,
        depth: _limits.depth,
        hardMaxChildren: _limits.hardMaxChildren,
        hardMaxTree: _limits.hardMaxTree,
      );
      _sessionsOverride = s;
    });
    final refused = await widget.onSaveLimits(c, tr, s);
    if (!mounted) return;
    setState(() {
      _saving = false;
      if (refused != null) {
        _limitsError = refused;
        _limits = was.$1;
        _sessionsOverride = was.$2;
      }
    });
  }

  /// What Sessions at once shows: the override, else what is in force.
  int? get _sessionsShown {
    if (_sessionsOverride != null) return _sessionsOverride;
    final e = _limits.effectiveMaxSessions ?? _limits.hostMaxSessions;
    return e == null || e == 0 ? null : e;
  }

  void _fewerSessions() {
    final base =
        _sessionsShown ??
        (widget.host.maxSessions > 0 ? widget.host.maxSessions : 9);
    _save(sessions: base - 1);
  }

  void _moreSessions() {
    final n = _sessionsOverride! + 1;
    final own = _limits.hostMaxSessions;
    // At the host's own number an override changes nothing: back to it.
    if (own != null && own > 0 && n >= own) {
      _save(clear: true);
    } else {
      _save(sessions: n);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final h = widget.host;
    final seen = DateTime.tryParse(h.lastSeen ?? '');
    final state = h.online
        ? 'online'
        : seen == null
        ? 'offline'
        : 'offline · last seen ${relativeTime(seen)}';
    final label = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text2,
    );
    final value = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text,
      height: 1.45,
    );
    final runsAs = h.executionLine.startsWith('Runs as ')
        ? h.executionLine.substring('Runs as '.length)
        : h.executionLine;
    // The execution line already says a verified sandbox isolates.
    final isolation =
        h.execution?.perSession == true &&
            h.isolationLabel == kIsolationVerified
        ? null
        : h.isolationLabel;
    final limits = h.limits;

    // On a phone the section's header already names the host, and a label
    // goes above its value.
    final narrow = !context.isExpanded;
    Widget pair(String k, Widget v, {double top = 0}) => narrow
        ? Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            spacing: t.sp * 0.5,
            children: [
              Text(k, style: label),
              v,
            ],
          )
        : Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: t.sp * 15,
                child: Padding(
                  padding: EdgeInsets.only(top: top),
                  child: Text(k, style: label),
                ),
              ),
              Expanded(child: v),
            ],
          );

    final heading = _renaming
        ? Row(
            spacing: t.sp * 1.5,
            children: [
              Expanded(
                child: TextField(
                  key: const Key('rename-host-field'),
                  controller: _name,
                  autofocus: true,
                  maxLength: 64,
                  decoration: const InputDecoration(counterText: ''),
                  onSubmitted: (v) async {
                    setState(() => _renaming = false);
                    await widget.onRename(v);
                  },
                ),
              ),
              TextAction(
                key: const Key('rename-host-save'),
                label: 'Save',
                accent: true,
                onTap: () async {
                  setState(() => _renaming = false);
                  await widget.onRename(_name.text);
                },
              ),
              TextAction(
                key: const Key('rename-host-cancel'),
                label: 'Cancel',
                onTap: () => setState(() {
                  _renaming = false;
                  _name.text = h.name;
                }),
              ),
            ],
          )
        : Row(
            crossAxisAlignment: CrossAxisAlignment.baseline,
            textBaseline: TextBaseline.alphabetic,
            spacing: t.sp * 1.25,
            children: [
              Expanded(
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.baseline,
                  textBaseline: TextBaseline.alphabetic,
                  spacing: t.sp * 1.25,
                  children: [
                    if (!narrow)
                      Flexible(
                        child: Text(
                          h.name,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontFamily: StormTokens.sansFamily,
                            fontSize: t.bodySize,
                            fontWeight: FontWeight.w600,
                            color: t.text,
                          ),
                        ),
                      ),
                    Text(
                      state,
                      key: Key('host-state-${h.id}'),
                      style: TextStyle(
                        fontFamily: StormTokens.monoFamily,
                        fontSize: t.labelSize,
                        color: t.text3,
                      ),
                    ),
                  ],
                ),
              ),
              TextAction(
                key: Key('rename-host-${h.id}'),
                label: 'Rename',
                onTap: () => setState(() {
                  _renaming = true;
                  _confirmRevoke = false;
                }),
              ),
              TextAction(
                key: Key('revoke-host-${h.id}'),
                label: 'Revoke…',
                danger: true,
                onTap: () => setState(() => _confirmRevoke = true),
              ),
            ],
          );

    return Column(
      key: Key('host-details-${h.id}'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: t.sp * 1.5,
      children: [
        heading,
        if (_confirmRevoke)
          Container(
            key: const Key('revoke-strip'),
            padding: EdgeInsets.all(t.sp * 1.5),
            decoration: BoxDecoration(
              color: t.danger.withValues(alpha: 0.08),
              borderRadius: BorderRadius.circular(t.rControl),
              border: Border.all(color: t.danger, width: t.bw),
            ),
            child: Wrap(
              spacing: t.sp * 1.5,
              runSpacing: t.sp,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                Text(
                  'Revoke ${h.name}? Its sessions end and it can no longer '
                  'connect. To use the machine again, re-enroll it.',
                  style: value,
                ),
                StormButton.danger(
                  key: const Key('revoke-host-confirm'),
                  label: 'Revoke',
                  onPressed: () async {
                    setState(() => _confirmRevoke = false);
                    await widget.onRevoke();
                  },
                ),
                StormButton.text(
                  key: const Key('revoke-host-cancel'),
                  label: 'Cancel',
                  onPressed: () => setState(() => _confirmRevoke = false),
                ),
              ],
            ),
          ),
        pair(
          'Runs as',
          Text(runsAs, key: Key('runs-as-${h.id}'), style: value),
        ),
        if (h.identityUnverified)
          pair('Identity', Text(kIdentityNotVerified, style: value)),
        if (isolation != null) pair('Isolation', Text(isolation, style: value)),
        if (limits != null)
          pair(
            'Limits',
            top: t.sp,
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              spacing: t.sp,
              children: [
                Wrap(
                  spacing: t.sp,
                  runSpacing: t.sp,
                  children: [
                    LimitStepper(
                      id: 'children',
                      label: 'Agents per session',
                      value: '${_limits.maxChildren}',
                      onLess: !_saving && _limits.maxChildren > 1
                          ? () => _save(children: _limits.maxChildren - 1)
                          : null,
                      onMore:
                          !_saving &&
                              _limits.maxChildren < _limits.hardMaxChildren
                          ? () => _save(children: _limits.maxChildren + 1)
                          : null,
                    ),
                    LimitStepper(
                      id: 'tree',
                      label: 'Per tree',
                      value: '${_limits.maxTree}',
                      onLess: !_saving && _limits.maxTree > 1
                          ? () => _save(tree: _limits.maxTree - 1)
                          : null,
                      onMore: !_saving && _limits.maxTree < _limits.hardMaxTree
                          ? () => _save(tree: _limits.maxTree + 1)
                          : null,
                    ),
                    LimitStepper(
                      id: 'sessions',
                      label: 'Sessions at once',
                      value: _sessionsShown?.toString() ?? 'any',
                      onLess: !_saving && (_sessionsShown ?? 2) > 1
                          ? _fewerSessions
                          : null,
                      onMore: !_saving && _sessionsOverride != null
                          ? _moreSessions
                          : null,
                    ),
                  ],
                ),
                if (_limits.overrideIneffective)
                  Text(
                    'Above the host’s own, so the host’s '
                    '${_limits.effectiveMaxSessions} is in force.',
                    key: const Key('limit-sessions-ineffective'),
                    style: label.copyWith(
                      fontSize: t.labelSize * 1.09,
                      color: t.text3,
                    ),
                  ),
                Text(
                  'Lowering a limit ends nothing that is running; it refuses '
                  'new starts.',
                  style: label.copyWith(
                    fontSize: t.labelSize * 1.09,
                    color: t.text3,
                  ),
                ),
                if (_limitsError != null)
                  Text(
                    _limitsError!,
                    key: const Key('limits-error'),
                    style: label.copyWith(color: t.danger),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

/// One limit: its label, which never wraps, then − value +.
class LimitStepper extends StatelessWidget {
  const LimitStepper({
    super.key,
    required this.id,
    required this.label,
    required this.value,
    required this.onLess,
    required this.onMore,
  });

  final String id;
  final String label;
  final String value;
  final VoidCallback? onLess;
  final VoidCallback? onMore;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    Widget step(String glyph, String tip, Key key, VoidCallback? onTap) =>
        Tooltip(
          message: tip,
          child: InkWell(
            key: key,
            onTap: onTap,
            borderRadius: BorderRadius.circular(t.rControl * 0.6),
            child: SizedBox(
              width: t.sp * 2.75,
              height: t.sp * 2.75,
              child: Center(
                child: Text(
                  glyph,
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.codeSize,
                    color: onTap == null
                        ? t.text3.withValues(alpha: 0.5)
                        : t.text3,
                  ),
                ),
              ),
            ),
          ),
        );
    return Container(
      key: Key('limit-$id-stepper'),
      padding: EdgeInsets.fromLTRB(
        t.sp * 1.5,
        t.sp * 0.25,
        t.sp * 0.5,
        t.sp * 0.25,
      ),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        spacing: t.sp * 1.25,
        children: [
          Flexible(
            child: Text(
              label,
              maxLines: 1,
              softWrap: false,
              overflow: TextOverflow.fade,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.text2,
              ),
            ),
          ),
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              step('−', 'Fewer', Key('limit-$id-less'), onLess),
              ConstrainedBox(
                constraints: BoxConstraints(minWidth: t.sp * 1.75),
                child: Text(
                  value,
                  key: Key('limit-$id'),
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.codeSize,
                    color: t.text,
                  ),
                ),
              ),
              step('+', 'More', Key('limit-$id-more'), onMore),
            ],
          ),
        ],
      ),
    );
  }
}

/// What a cell's popover (or, on a phone, its sheet) holds: what the agent
/// is on that host, what that means for sessions, and the one fix.
class CellPanel extends StatefulWidget {
  const CellPanel({
    super.key,
    required this.host,
    required this.agent,
    required this.run,
    required this.hostBusy,
    required this.canType,
    required this.isDefault,
    required this.onAction,
    required this.onSignIn,
    required this.onOpenShell,
    required this.onMakeDefault,
    this.sheet = false,
  });

  final AgentHost host;
  final String agent;
  final AgentRun? run;

  /// The host is installing, updating or removing something: one at a time.
  final bool hostBusy;

  /// The host can open a Shell with a command typed in (153-S3).
  final bool canType;
  final bool isDefault;
  final ValueChanged<String> onAction;

  /// Opens a Shell with the command typed in; answers why not.
  final Future<String?> Function(String command) onSignIn;
  final Future<String?> Function() onOpenShell;
  final VoidCallback onMakeDefault;

  /// In the phone's sheet, whose title already names it.
  final bool sheet;

  @override
  State<CellPanel> createState() => _CellPanelState();
}

class _CellPanelState extends State<CellPanel> {
  bool _confirmRemove = false;
  bool _busy = false;
  bool _copied = false;
  String? _error;

  Future<void> _shell(Future<String?> Function() open) async {
    setState(() {
      _busy = true;
      _error = null;
    });
    final why = await open();
    if (!mounted) return;
    setState(() {
      _busy = false;
      _error = why;
    });
  }

  Future<void> _copy(String command) async {
    final ok = await copyToClipboard(command);
    if (mounted) setState(() => _copied = ok);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final h = widget.host;
    final agent = widget.agent;
    final name = providerLabel(agent);
    final p = capFor(h, agent);
    final cell = agentCell(h, agent, widget.run);
    final installed = p != null && isInstalled(p);
    final small = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.labelSize * 1.09,
      color: t.text3,
      height: 1.5,
    );
    final body = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text,
      height: 1.5,
    );
    final mono = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
      color: t.text3,
    );

    final meta = [
      if (!h.online)
        'host offline'
      else if (p == null || !installed)
        'not installed'
      else ...[
        ?p.version,
        switch (p.source) {
          'storm' => 'installed by Storm',
          'system' => 'your own install',
          _ => 'installed',
        },
        if (hasUpdate(p))
          '${p.latest} available'
        else if (p.managed && p.latest != null && p.latest == p.version)
          'up to date',
      ],
    ].join(' · ');

    final meaning = switch (cell.word) {
      CellWord.offline =>
        '${h.name} is offline. Its agents can be changed once it reconnects.',
      CellWord.busy =>
        '${widget.run!.progress} Sessions on ${h.name} can use $name once '
            "it's done.",
      CellWord.install =>
        "Not installed. Sessions can't use $name on ${h.name} until it is.",
      CellWord.cantInstall =>
        p == null || !h.agentInstall
            ? 'Update storm-runtime on ${h.name} to manage agents there.'
            : '${installBlockedText(p.installBlocked, agent)}. Sessions '
                  "can't use $name there.",
      CellWord.update =>
        '${p!.latest} is available. Sessions keep using ${p.version} until '
            'you update.',
      CellWord.signIn =>
        "Not signed in. Sessions can't use $name on ${h.name} until it is.",
      CellWord.ready =>
        p?.signedIn == null
            ? "Sessions on ${h.name} can use $name. Storm can't see whether "
                  "it's signed in."
            : 'Signed in. Sessions on ${h.name} can use $name.',
    };

    final command = p?.loginCommand;
    final offerSignIn =
        command != null &&
        installed &&
        h.online &&
        (cell.word == CellWord.signIn ||
            (cell.word == CellWord.ready && p.signedIn == null));
    final urgent = cell.word == CellWord.signIn;

    Widget commandBox(String command) => Container(
      key: const Key('signin-command'),
      padding: EdgeInsets.only(left: t.sp),
      decoration: BoxDecoration(
        color: t.bg,
        borderRadius: BorderRadius.circular(t.rControl),
      ),
      child: Row(
        children: [
          Expanded(
            child: SelectableText(
              command,
              style: mono.copyWith(color: t.text2),
            ),
          ),
          TextButton(
            key: const Key('signin-copy'),
            onPressed: () => _copy(command),
            child: Text(
              _copied ? 'Copied' : 'Copy',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.labelSize * 1.09,
                color: t.accent,
              ),
            ),
          ),
        ],
      ),
    );

    final children = <Widget>[
      if (!widget.sheet)
        Text(
          '$name on ${h.name}',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.uiSize,
            fontWeight: FontWeight.w600,
            color: t.text,
          ),
        ),
      Text(meta, key: const Key('cell-meta'), style: mono),
      Text(meaning, key: const Key('cell-meaning'), style: body),
      if (cell.word == CellWord.install && h.agentInstall)
        Align(
          alignment: Alignment.centerLeft,
          child: StormButton.primary(
            key: const Key('cell-fix'),
            label: 'Install',
            onPressed: widget.hostBusy
                ? null
                : () => widget.onAction('install'),
          ),
        ),
      if (cell.word == CellWord.update)
        Align(
          alignment: Alignment.centerLeft,
          child: StormButton.primary(
            key: const Key('cell-fix'),
            label: 'Update to ${p!.latest}',
            onPressed: widget.hostBusy ? null : () => widget.onAction('update'),
          ),
        ),
      if (offerSignIn) ...[
        if (h.execution?.perSession == true) ...[
          Text(
            'Each session on this host runs as its own user, so sign in on '
            'the host itself: $command',
            key: const Key('signin-on-host'),
            style: body,
          ),
          commandBox(command),
        ] else if (!widget.canType) ...[
          Text(
            "${h.name}'s storm-runtime can't type a command in for you. "
            'Open a Shell there, then paste this and press Enter:',
            key: const Key('signin-fallback'),
            style: small,
          ),
          commandBox(command),
          Align(
            alignment: Alignment.centerLeft,
            child: StormButton.outline(
              key: const Key('open-shell'),
              label: 'Open a Shell on ${h.name}',
              onPressed: _busy ? null : () => _shell(widget.onOpenShell),
            ),
          ),
        ] else ...[
          Align(
            alignment: Alignment.centerLeft,
            child: StormButton(
              key: const Key('cell-signin'),
              kind: urgent ? StormButtonKind.primary : StormButtonKind.outline,
              label: 'Sign in in a Shell',
              onPressed: _busy
                  ? null
                  : () => _shell(() => widget.onSignIn(command)),
            ),
          ),
          Text.rich(
            TextSpan(
              children: [
                TextSpan(text: 'Opens a Shell session on ${h.name} with '),
                TextSpan(
                  text: command,
                  style: mono.copyWith(color: t.text2),
                ),
                const TextSpan(text: ' typed in. Press Enter to run it.'),
              ],
            ),
            style: small,
          ),
        ],
      ],
      if (_error != null)
        Text(
          _error!,
          key: const Key('cell-error'),
          style: small.copyWith(color: t.danger),
        ),
      if (widget.run?.error case final error?)
        Text(
          error,
          key: const Key('agent-error'),
          style: small.copyWith(color: t.danger),
        ),
    ];

    final notes = <Widget>[
      if (installed && p.source == 'system')
        Text(
          "Installed outside Storm, so Storm won't update or remove it.",
          key: const Key('outside-storm'),
          style: small,
        ),
      if (h.online && !h.agentInstall && p != null)
        Text('Update storm-runtime to manage agents here.', style: small),
      if (h.online && p != null && !p.checksumVerified)
        Text('Not checksum-verified by Cursor.', style: small),
      if (installed &&
          p.managed &&
          h.online &&
          h.agentInstall &&
          cell.word != CellWord.busy)
        _confirmRemove
            ? Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                spacing: t.sp * 0.75,
                children: [
                  Text(
                    'Remove $name from ${h.name}? Storm deletes the copy it '
                    'installed. Its sign-in and settings on the host stay.',
                    style: small.copyWith(color: t.text2),
                  ),
                  RowActions(
                    children: [
                      TextAction(
                        key: const Key('agent-remove-confirm'),
                        label: 'Remove',
                        danger: true,
                        onTap: widget.hostBusy
                            ? null
                            : () {
                                setState(() => _confirmRemove = false);
                                widget.onAction('remove');
                              },
                      ),
                      TextAction(
                        label: 'Cancel',
                        onTap: () => setState(() => _confirmRemove = false),
                      ),
                    ],
                  ),
                ],
              )
            : Align(
                alignment: Alignment.centerLeft,
                child: TextAction(
                  key: const Key('cell-remove'),
                  label: 'Remove from ${h.name}',
                  onTap: widget.hostBusy
                      ? null
                      : () => setState(() => _confirmRemove = true),
                ),
              ),
      if (widget.sheet && !widget.isDefault)
        Align(
          alignment: Alignment.centerLeft,
          child: TextAction(
            key: const Key('make-default'),
            label: 'Make $name the default',
            accent: true,
            onTap: widget.onMakeDefault,
          ),
        ),
    ];

    return Column(
      key: const Key('cell-panel'),
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: t.sp * 1.25,
      children: [
        ...children,
        if (notes.isNotEmpty) ...[
          Container(height: t.bw, color: t.border),
          ...notes,
        ],
      ],
    );
  }
}

/// A host on a phone: its header, then one row per agent; tapping a row
/// opens the cell's sheet. There is no grid on a phone.
class HostSection extends StatelessWidget {
  const HostSection({
    super.key,
    required this.host,
    required this.defaultProvider,
    required this.runs,
    required this.expanded,
    required this.hostAdmin,
    required this.details,
    required this.onSelect,
    required this.onOpen,
    required this.onReenroll,
    required this.onRemove,
  });

  final AgentHost host;
  final String? defaultProvider;
  final Map<String, AgentRun> runs;

  /// Its details are open under it.
  final bool expanded;
  final bool hostAdmin;
  final Widget? details;
  final VoidCallback onSelect;
  final ValueChanged<String> onOpen;
  final VoidCallback onReenroll;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final h = host;
    final revoked = h.status == 'revoked';
    return Container(
      margin: EdgeInsets.only(bottom: t.sp * 2),
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        color: t.surface,
        borderRadius: BorderRadius.circular(t.rCard),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Opacity(
            opacity: revoked ? 0.75 : 1,
            child: InkWell(
              key: Key('host-cell-${h.id}'),
              onTap: revoked ? null : onSelect,
              child: Padding(
                padding: EdgeInsets.all(t.sp * 2),
                child: HostLabel(host: h),
              ),
            ),
          ),
          if (revoked)
            Padding(
              padding: EdgeInsets.fromLTRB(t.sp * 2, 0, t.sp * 2, t.sp * 2),
              child: RevokedActions(
                host: h,
                hostAdmin: hostAdmin,
                onReenroll: onReenroll,
                onRemove: onRemove,
              ),
            )
          else
            for (final a in kInstallableAgents)
              _PhoneAgentRow(
                host: h,
                agent: a,
                cell: agentCell(h, a, runs['${h.id}/$a']),
                isDefault: a == defaultProvider,
                onTap: () => onOpen(a),
              ),
          if (expanded && details != null)
            Container(
              color: t.surface2,
              padding: EdgeInsets.all(t.sp * 1.5),
              child: details,
            ),
        ],
      ),
    );
  }
}

class _PhoneAgentRow extends StatelessWidget {
  const _PhoneAgentRow({
    required this.host,
    required this.agent,
    required this.cell,
    required this.isDefault,
    required this.onTap,
  });

  final AgentHost host;
  final String agent;
  final AgentCell cell;
  final bool isDefault;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final id = '${host.id}-$agent';
    return InkWell(
      key: Key('agent-row-$id'),
      onTap: onTap,
      child: Container(
        decoration: BoxDecoration(
          border: Border(
            top: BorderSide(color: t.border, width: t.bw),
          ),
        ),
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 2,
          vertical: t.sp * 1.5,
        ),
        child: Row(
          spacing: t.sp,
          children: [
            Expanded(
              child: Wrap(
                spacing: t.sp,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text(
                    providerLabel(agent),
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize,
                      color: t.text,
                    ),
                  ),
                  if (isDefault) DefaultTag(key: Key('default-tag-$agent')),
                ],
              ),
            ),
            Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              spacing: t.sp * 0.25,
              children: [
                Text(
                  cell.top,
                  key: Key('cell-word-$id'),
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    fontWeight: cell.actionable
                        ? FontWeight.w500
                        : FontWeight.w400,
                    color: cellWordColor(t, cell),
                  ),
                ),
                if (cell.progress != null)
                  ProgressBar(key: Key('cell-bar-$id'), value: cell.progress!),
                if (cell.sub.isNotEmpty)
                  Text(
                    cell.sub,
                    key: Key('cell-sub-$id'),
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: t.text3,
                    ),
                  ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// No hosts yet: one dashed card instead of the same sentence under five
/// agents.
class NoHostsCard extends StatelessWidget {
  const NoHostsCard({super.key, required this.onEnroll});

  final VoidCallback? onEnroll;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return CustomPaint(
      key: const Key('hosts-empty'),
      painter: _Dashed(color: t.border, radius: t.rCard, width: t.bw),
      child: Padding(
        padding: EdgeInsets.all(t.sp * 3.5),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          spacing: t.sp * 1.25,
          children: [
            Text(
              'No hosts yet',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.bodySize,
                fontWeight: FontWeight.w600,
                color: t.text,
              ),
            ),
            ConstrainedBox(
              constraints: BoxConstraints(maxWidth: t.sp * 60),
              child: Text(
                'Agents run on a machine you own. Enroll one, then install '
                'Claude Code, OpenCode, Codex, Cursor or Command Code on it '
                'from this page.',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.uiSize,
                  color: t.text2,
                  height: 1.55,
                ),
              ),
            ),
            SizedBox(height: t.sp * 0.5),
            StormButton.primary(
              key: const Key('enroll-host-empty'),
              label: 'Enroll a host',
              onPressed: onEnroll,
            ),
          ],
        ),
      ),
    );
  }
}

class _Dashed extends CustomPainter {
  const _Dashed({
    required this.color,
    required this.radius,
    required this.width,
  });

  final Color color;
  final double radius;
  final double width;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = width;
    final path = Path()
      ..addRRect(
        RRect.fromRectAndRadius(
          (Offset.zero & size).deflate(width / 2),
          Radius.circular(radius),
        ),
      );
    final dash = width * 3;
    for (final metric in path.computeMetrics()) {
      for (var d = 0.0; d < metric.length; d += dash * 2) {
        canvas.drawPath(metric.extractPath(d, d + dash), paint);
      }
    }
  }

  @override
  bool shouldRepaint(_Dashed old) =>
      old.color != color || old.radius != radius || old.width != width;
}
