import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';
import '../state/health.dart' show integrationsSummaryProvider;
import '../ui/controls.dart';
import '../ui/states.dart';
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart' show PopoverItem;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';

typedef NoteRef = ({String vaultId, String noteId});

/// What the launcher starts from: nothing, a note, an agent card, or an
/// ended session to run again (handoff §3.5, §3.7).
class LaunchPrefill {
  const LaunchPrefill({
    this.hostId,
    this.workspace,
    this.provider,
    this.context,
    this.contextTitle,
    this.writes,
    this.writeVaultId,
  });

  /// Everything the session was launched with. Its host may be offline by
  /// now; the launcher then falls back to the first online one.
  factory LaunchPrefill.runAgain(AgentSession s) => LaunchPrefill(
    hostId: s.hostId,
    workspace: s.workspace,
    provider: s.provider,
    context: s.context == null
        ? null
        : (vaultId: s.context!.vaultId, noteId: s.context!.noteId),
    contextTitle: s.context?.title,
    writes: s.writeVaultId != null,
    writeVaultId: s.writeVaultId,
  );

  final String? hostId;
  final String? workspace;
  final String? provider;
  final NoteRef? context;
  final String? contextTitle;

  /// Null: on when Settings allows agent writes (handoff §4.5).
  final bool? writes;
  final String? writeVaultId;
}

/// The New session launcher: a centred modal at desk width, a bottom sheet
/// on a phone. Pops the launched session, or nothing.
class NewSessionLauncher extends ConsumerStatefulWidget {
  const NewSessionLauncher({
    super.key,
    required this.hosts,
    this.prefill = const LaunchPrefill(),
  });

  final List<AgentHost> hosts;
  final LaunchPrefill prefill;

  @override
  ConsumerState<NewSessionLauncher> createState() => _LauncherState();
}

class _LauncherState extends ConsumerState<NewSessionLauncher> {
  AgentHost? _host;
  List<AgentWorkspace>? _workspaces;
  String? _workspace;
  String? _provider;

  /// The agent asked for — by Run again, a card or a pick — over the
  /// default, wherever the host offers it.
  String? _chosen;
  String? _default;
  NoteRef? _context;
  bool? _writes;
  String? _writeVault;
  String? _error;
  bool _busy = false;

  List<AgentHost> get _online => [
    for (final h in widget.hosts)
      if (h.online) h,
  ];

  @override
  void initState() {
    super.initState();
    final p = widget.prefill;
    _context = p.context;
    _writes = p.writes;
    _writeVault = p.writeVaultId ?? p.context?.vaultId;
    _chosen = p.provider;
    _preselectHost();
    _loadDefault();
  }

  /// The session's own host when running one again, else the one this
  /// device launched on last, else the first online (decision 78).
  Future<void> _preselectHost() async {
    final online = _online;
    if (online.isEmpty) return;
    final wanted = widget.prefill.hostId ?? await lastLaunchHost();
    if (!mounted || _host != null) return;
    await _pickHost(
      online.where((h) => h.id == wanted).firstOrNull ?? online.first,
      keepWorkspace: widget.prefill.workspace,
    );
  }

  Future<void> _loadDefault() async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final d = await api.defaultProvider();
      if (!mounted) return;
      setState(() {
        _default = d;
        _settleProvider();
      });
    } catch (_) {
    } finally {
      api.dispose();
    }
  }

  /// Keeps the chosen agent if this host offers it, else the default, else
  /// the first one installed.
  void _settleProvider() {
    final host = _host;
    if (host == null) return;
    final installed = [
      for (final p in host.providers)
        if (p.available) p.id,
    ];
    _provider = installed.contains(_chosen)
        ? _chosen
        : installed.contains(_default)
        ? _default
        : installed.firstOrNull;
  }

  Future<void> _pickHost(AgentHost host, {String? keepWorkspace}) async {
    setState(() {
      _host = host;
      _workspaces = null;
      _workspace = null;
      _settleProvider();
    });
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final ws = await api.workspaces(host.id);
      if (!mounted || _host?.id != host.id) return;
      setState(() {
        _workspaces = ws;
        _workspace =
            ws.where((w) => w.name == keepWorkspace).firstOrNull?.name ??
            ws.firstOrNull?.name;
      });
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
    }
  }

  Future<void> _launch(bool writes, String? writeVault) async {
    final host = _host, ws = _workspace;
    if (host == null || ws == null) return;
    final api = agentApi(ref);
    if (api == null) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final size = MediaQuery.sizeOf(context);
      final session = await api.launch(
        hostId: host.id,
        workspace: ws,
        provider: _provider,
        // A first guess; the terminal sends its real size once laid out.
        cols: (size.width / 9).clamp(40, 240).floor(),
        rows: (size.height / 20).clamp(12, 80).floor(),
        context: _context,
        writeVaultId: writes && _provider != 'shell' ? writeVault : null,
      );
      await rememberLaunchHost(host.id);
      if (!mounted) return;
      final notice = session.launchNotice;
      if (notice != null) {
        ScaffoldMessenger.maybeOf(
          context,
        )?.showSnackBar(SnackBar(content: Text(notice)));
      }
      Navigator.of(context).pop(session);
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final host = _host;
    final shell = _provider == 'shell';
    final allowed = ref.watch(agentWritesSettingProvider).value == true;
    final vaults = [
      for (final v in ref.watch(vaultsProvider).value ?? const <VaultInfo>[])
        if (!v.missing) v,
    ];
    final writeVault =
        vaults.where((v) => v.id == _writeVault).firstOrNull?.id ??
        vaults.firstOrNull?.id;
    final writes = allowed && (_writes ?? true) && writeVault != null;
    final integrations = [
      for (final i
          in ref.watch(integrationsSummaryProvider).value ?? const <Never>[])
        if (!i.builtin && !i.disabled) i.displayName,
    ];
    final hostName = host?.name ?? 'the host';
    final ctx = _context;

    final children = <Widget>[
      Row(
        children: [
          Expanded(
            child: Text(
              'New session',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.headingSize * 0.9,
                fontWeight: FontWeight.w600,
                color: t.text,
              ),
            ),
          ),
          _Close(onTap: () => Navigator.of(context).pop()),
        ],
      ),
      if (ctx != null)
        _ContextBox(
          at: ctx,
          title: widget.prefill.context == ctx
              ? widget.prefill.contextTitle
              : null,
          vaults: vaults,
          onClear: () => setState(() => _context = null),
        ),
      if (_online.isEmpty)
        Text(
          'No host is online. Start storm-runtime on one, or enroll a host '
          'in Settings › Hosts.',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.text3,
          ),
        )
      else ...[
        _Field(
          label: 'Host',
          child: _Select<String>(
            key: const Key('launcher-host'),
            label: 'Host',
            value: host?.id,
            options: [for (final h in _online) (h.id, '${h.name} · online')],
            onChanged: (id) => _pickHost(_online.firstWhere((h) => h.id == id)),
          ),
        ),
        _Field(
          label: 'Workspace',
          child: _Select<String>(
            key: const Key('launcher-workspace'),
            label: 'Workspace',
            value: _workspace,
            placeholder: _workspaces == null
                ? '…'
                : 'No workspaces on this host',
            options: [
              for (final w in _workspaces ?? const <AgentWorkspace>[])
                (
                  w.name,
                  w.liveSessions > 0
                      ? '${w.name}  (another session is working here)'
                      : w.name,
                ),
            ],
            onChanged: (w) => setState(() => _workspace = w),
          ),
        ),
        _Field(
          label: 'Agent',
          child: _Select<String>(
            key: const Key('launcher-agent'),
            label: 'Agent',
            value: _provider,
            options: [
              for (final p in host?.providers ?? const <ProviderCap>[])
                if (p.available)
                  (p.id, p.id == _default ? '${p.label} · default' : p.label),
            ],
            disabled: [
              for (final p in host?.providers ?? const <ProviderCap>[])
                if (!p.available) '${p.label} (not installed)',
            ],
            onChanged: (p) => setState(() => _provider = _chosen = p),
          ),
        ),
        if (!shell)
          _Field(
            label: 'Can write to',
            child: Row(
              children: [
                Semantics(
                  label: 'Can write to',
                  child: StormToggle(
                    key: const Key('launcher-writes'),
                    value: writes,
                    enabled: allowed && writeVault != null,
                    onChanged: (v) => setState(() => _writes = v),
                  ),
                ),
                SizedBox(width: t.sp * 1.25),
                Expanded(
                  child: writes
                      ? _Select<String>(
                          key: const Key('launcher-write-vault'),
                          label: 'Write vault',
                          value: writeVault,
                          compact: true,
                          options: [for (final v in vaults) (v.id, v.name)],
                          onChanged: (v) => setState(() => _writeVault = v),
                        )
                      : Text(
                          allowed ? 'Read only' : 'Off in Settings › AI access',
                          key: const Key('launcher-read-only'),
                          style: TextStyle(
                            fontFamily: StormTokens.sansFamily,
                            fontSize: t.codeSize,
                            color: t.text3,
                          ),
                        ),
                ),
              ],
            ),
          ),
      ],
      Container(
        key: const Key('launcher-risk'),
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 1.5,
          vertical: t.sp * 1.25,
        ),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl),
        ),
        child: Text(
          shell
              ? 'Network: inherits $hostName’s policy. A shell has no access to '
                    'your vaults or integrations.'
              : integrations.isEmpty
              ? 'Network: inherits $hostName’s policy. This session can use '
                    'your integrations and read your vaults, with that network '
                    'access.'
              : 'Network: inherits $hostName’s policy. This session can use '
                    'your integrations (${integrations.join(', ')}) and read '
                    'your vaults, with that network access.',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.labelSize * 1.09,
            height: 1.55,
            color: t.text3,
          ),
        ),
      ),
      if (_error != null)
        Text(
          _error!,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.danger,
          ),
        ),
      Row(
        children: [
          Expanded(
            child: ctx == null
                ? const SizedBox.shrink()
                : Text(
                    'The agent reads the note through Storm.',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.labelSize * 1.09,
                      color: t.text3,
                    ),
                  ),
          ),
          StormButton.primary(
            key: const Key('launch'),
            label: _busy ? 'Starting' : 'Launch',
            onPressed: _busy || host == null || _workspace == null
                ? null
                : () => _launch(writes, writeVault),
          ),
        ],
      ),
    ];

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (var i = 0; i < children.length; i++) ...[
          if (i > 0) SizedBox(height: t.sp * 1.75),
          children[i],
        ],
      ],
    );
  }
}

class _Close extends StatelessWidget {
  const _Close({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      button: true,
      label: 'Close',
      excludeSemantics: true,
      onTap: onTap,
      child: InkWell(
        key: const Key('launcher-close'),
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl),
        child: Padding(
          padding: EdgeInsets.all(t.sp * 0.5),
          child: Icon(LucideIcons.x, size: t.bodySize, color: t.text3),
        ),
      ),
    );
  }
}

/// CONTEXT, then the note in an `accentSoft` box with its crumb and ×.
class _ContextBox extends ConsumerWidget {
  const _ContextBox({
    required this.at,
    required this.title,
    required this.vaults,
    required this.onClear,
  });

  final NoteRef at;
  final String? title;
  final List<VaultInfo> vaults;
  final VoidCallback onClear;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final note = ref.watch(agentNoteProvider(at)).value;
    final vault =
        vaults.where((v) => v.id == at.vaultId).firstOrNull?.name ?? '';
    // The launch snapshot when there is one: a rename does not change what
    // a session was started from.
    final shown =
        title ??
        (note == null ? '' : noteTitleOf(note.meta.title, note.meta.path));
    return Column(
      key: const Key('launcher-context'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        const AgentsLabel('Context'),
        SizedBox(height: t.sp * 0.75),
        Container(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 1.5,
            vertical: t.sp * 1.25,
          ),
          decoration: BoxDecoration(
            color: t.accentSoft,
            borderRadius: BorderRadius.circular(t.rControl),
            border: Border.all(color: t.border, width: t.bw),
          ),
          child: Row(
            children: [
              Icon(LucideIcons.file, size: t.uiSize, color: t.accent),
              SizedBox(width: t.sp * 1.25),
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: t.sp * 25),
                child: Text(
                  shown,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.uiSize,
                    color: t.text,
                  ),
                ),
              ),
              if (note != null) ...[
                SizedBox(width: t.sp),
                Expanded(
                  child: Text(
                    noteCrumb(vault, note.meta.path),
                    textAlign: TextAlign.right,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: t.text3,
                    ),
                  ),
                ),
              ],
              SizedBox(width: t.sp),
              Semantics(
                button: true,
                label: 'Remove context',
                excludeSemantics: true,
                onTap: onClear,
                child: InkWell(
                  key: const Key('launcher-context-clear'),
                  onTap: onClear,
                  child: Icon(LucideIcons.x, size: t.codeSize, color: t.text3),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

/// A 96px label column and the control beside it.
class _Field extends StatelessWidget {
  const _Field({required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Row(
      children: [
        SizedBox(
          width: t.sp * 12,
          child: Text(
            label,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.text2,
            ),
          ),
        ),
        SizedBox(width: t.sp * 1.25),
        Expanded(child: child),
      ],
    );
  }
}

/// A select: `surface2`, a hairline, `rControl`, and a popover of options.
class _Select<T> extends StatefulWidget {
  const _Select({
    super.key,
    required this.label,
    required this.value,
    required this.options,
    required this.onChanged,
    this.disabled = const [],
    this.placeholder = '',
    this.compact = false,
  });

  final String label;
  final T? value;
  final List<(T, String)> options;
  final ValueChanged<T> onChanged;

  /// Shown in the menu, never choosable (an agent not installed).
  final List<String> disabled;
  final String placeholder;
  final bool compact;

  @override
  State<_Select<T>> createState() => _SelectState<T>();
}

class _SelectState<T> extends State<_Select<T>> {
  final _anchor = GlobalKey();

  Future<void> _open() async {
    final box = _anchor.currentContext?.findRenderObject() as RenderBox?;
    final picked = await showStormPopover<T>(
      context: context,
      anchorKey: _anchor,
      width: box?.size.width ?? 240,
      builder: (popover) => StormPopover(
        children: [
          for (final (value, label) in widget.options)
            PopoverItem(
              label: label,
              selected: value == widget.value,
              onTap: () => Navigator.of(popover).pop(value),
            ),
          for (final label in widget.disabled) PopoverItem(label: label),
        ],
      ),
    );
    if (picked != null && picked != widget.value) widget.onChanged(picked);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final shown =
        widget.options.where((o) => o.$1 == widget.value).firstOrNull?.$2 ??
        widget.placeholder;
    final live = widget.options.isNotEmpty;
    return Semantics(
      button: true,
      label: '${widget.label}: $shown',
      excludeSemantics: true,
      onTap: live ? _open : null,
      child: Material(
        key: _anchor,
        color: t.surface2,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(t.rControl),
          side: BorderSide(color: t.border, width: t.bw),
        ),
        child: InkWell(
          onTap: live ? _open : null,
          borderRadius: BorderRadius.circular(t.rControl),
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.5,
              vertical: t.sp * (widget.compact ? 0.875 : 1.125),
            ),
            child: Text(
              shown,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.uiSize,
                color: live ? t.text : t.text3,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Raises the launcher in the shape the width calls for: a 460 modal at
/// [kExpandedWidth] and wider, a bottom sheet below it.
Future<AgentSession?> showNewSessionLauncher(
  BuildContext context, {
  required List<AgentHost> hosts,
  required bool wide,
  LaunchPrefill prefill = const LaunchPrefill(),
}) {
  final t = context.tokens;
  final launcher = NewSessionLauncher(hosts: hosts, prefill: prefill);
  if (!wide) {
    return showModalBottomSheet<AgentSession>(
      context: context,
      isScrollControlled: true,
      backgroundColor: t.surface,
      barrierColor: const Color(0x59000000),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(t.rCard)),
        side: BorderSide(color: t.border, width: t.bw),
      ),
      builder: (sheet) => Padding(
        padding: EdgeInsets.only(bottom: MediaQuery.viewInsetsOf(sheet).bottom),
        child: SafeArea(
          top: false,
          child: SingleChildScrollView(
            padding: EdgeInsets.fromLTRB(
              t.sp * 2.5,
              t.sp * 2.5,
              t.sp * 2.5,
              t.sp * 4,
            ),
            child: launcher,
          ),
        ),
      ),
    );
  }
  return showGeneralDialog<AgentSession>(
    context: context,
    barrierDismissible: true,
    barrierLabel: 'Close',
    barrierColor: const Color(0x66000000),
    transitionDuration: t.duration,
    pageBuilder: (dialog, _, _) => Align(
      alignment: Alignment.topCenter,
      child: Padding(
        padding: EdgeInsets.only(top: t.sp * 12),
        child: SizedBox(
          width: t.sp * 57.5,
          child: DecoratedBox(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(t.rCard),
              boxShadow: t.shadow,
            ),
            child: Material(
              color: t.surface,
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(t.rCard),
                side: BorderSide(color: t.border, width: t.bw),
              ),
              child: Padding(
                padding: EdgeInsets.all(t.sp * 2.75),
                child: launcher,
              ),
            ),
          ),
        ),
      ),
    ),
  );
}
