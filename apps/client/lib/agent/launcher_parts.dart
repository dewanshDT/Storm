part of 'launcher.dart';

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

/// From: the context note as one `accentSoft` chip with its crumb and ×.
class _ContextChip extends ConsumerWidget {
  const _ContextChip({
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
    return Container(
      key: const Key('launcher-context'),
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 1.5,
        vertical: t.sp * 1.125,
      ),
      decoration: BoxDecoration(
        color: t.accentSoft,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Row(
        children: [
          Icon(LucideIcons.file, size: t.codeSize, color: t.accent),
          SizedBox(width: t.sp),
          Flexible(
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
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.labelSize * 1.09,
                  color: t.text2,
                ),
              ),
            ),
          ] else
            const Spacer(),
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
    );
  }
}

/// Which Claude login the session will use (decision 124, spec §4), by the
/// server's own rules, under the Agent field.
class _ClaudeLoginLine extends StatelessWidget {
  const _ClaudeLoginLine({required this.login, required this.host});

  final ClaudeLogin login;
  final AgentHost host;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final route = claudeLoginRoute(login, host);
    // Decision 136f: an isolated session never uses the host's own login.
    final own = host.isolationState == 'verified'
        ? 'Claude: needs your synced Claude login or an API key, since this '
              'host isolates sessions'
        : "Claude: this host's own login";
    final soon =
        route == ClaudeLoginRoute.synced &&
        login.warn == '7d' &&
        login.daysLeft != null;
    final text = switch (route) {
      ClaudeLoginRoute.synced =>
        soon
            ? 'Claude: your synced login · expires in ${login.daysLeft} '
                  '${login.daysLeft == 1 ? 'day' : 'days'}, replace it in Settings'
            : 'Claude: your synced login',
      ClaudeLoginRoute.own => own,
      ClaudeLoginRoute.oldHost =>
        "$own · ${host.name} can't receive your synced login, update "
            'storm-runtime',
      ClaudeLoginRoute.expired =>
        '$own · your synced login has expired, re-sync it in Settings',
      ClaudeLoginRoute.needsResync =>
        "$own · your synced login can't be read, sync it again in Settings",
    };
    final warn =
        soon ||
        route == ClaudeLoginRoute.expired ||
        route == ClaudeLoginRoute.needsResync ||
        route == ClaudeLoginRoute.oldHost;
    return Text(
      text,
      key: Key('launcher-claude-login-${route.name}'),
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.labelSize * 1.09,
        height: 1.45,
        color: warn ? t.danger : t.text3,
      ),
    );
  }
}

/// A host with nothing to work in yet: where a workspace goes, and a way to
/// look again once one is there.
class _NoWorkspaces extends StatelessWidget {
  const _NoWorkspaces({required this.host, required this.onRetry});

  final AgentHost host;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final root = host.workspaceRoots.firstOrNull;
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text3,
    );
    return Column(
      key: const Key('launcher-no-workspaces'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          'A workspace is a folder in the host\'s workspaces folder. '
          'Make one, or clone a repository there, on ${host.name}:',
          style: style,
        ),
        SizedBox(height: t.sp * 0.75),
        SelectableText(
          root ?? 'the workspace_roots folder in its config.toml',
          style: style.copyWith(
            fontFamily: StormTokens.monoFamily,
            color: t.text2,
          ),
        ),
        SizedBox(height: t.sp * 0.75),
        StormButton.text(
          key: const Key('launcher-workspaces-retry'),
          label: 'Check again',
          onPressed: onRetry,
        ),
      ],
    );
  }
}

/// One launcher row: its label beside the control at desk width, above it
/// in the phone's sheet, and a line under the control.
class _Field extends StatelessWidget {
  const _Field({
    super.key,
    required this.label,
    required this.child,
    required this.stacked,
    this.under,
    this.underGap = true,
    this.anchor,
  });

  /// Null: the control takes the whole row.
  final String? label;
  final Widget child;
  final bool stacked;

  /// A line under [child], lined up with it.
  final Widget? under;
  final bool underGap;

  /// Where a chip scrolls to.
  final GlobalKey? anchor;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final name = label == null
        ? null
        : Text(
            label!,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.text2,
            ),
          );
    final indent = stacked || label == null ? 0.0 : t.sp * 15.25;
    final Widget row = stacked || name == null
        ? Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (name != null) ...[name, SizedBox(height: t.sp * 0.75)],
              child,
            ],
          )
        : Row(
            children: [
              SizedBox(width: t.sp * 14, child: name),
              SizedBox(width: t.sp * 1.25),
              Expanded(child: child),
            ],
          );
    final body = under == null
        ? row
        : Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              row,
              Padding(
                padding: EdgeInsets.only(
                  left: indent,
                  top: underGap ? t.sp * 0.75 : 0,
                ),
                child: under,
              ),
            ],
          );
    return anchor == null ? body : KeyedSubtree(key: anchor, child: body);
  }
}

/// A quiet line under a field's control.
class _Under extends StatelessWidget {
  const _Under({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      text,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.labelSize * 1.09,
        height: 1.45,
        color: t.text3,
      ),
    );
  }
}

/// An `accent` text action: More, Fewer options, New workspace…. As wide
/// as its words, so a row's empty end is not a target.
class _Link extends StatelessWidget {
  const _Link({super.key, required this.text, required this.onTap});

  final String text;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      button: true,
      label: text,
      excludeSemantics: true,
      onTap: onTap,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl),
        child: Padding(
          padding: EdgeInsets.symmetric(vertical: t.sp * 0.25),
          child: Text(
            text,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.accent,
            ),
          ),
        ),
      ),
    );
  }
}

enum LauncherChipState { on, off, unavailable }

/// One of "This session can"'s chips (153-5): an `accent` outline when on,
/// neutral when off, muted when the agent can't have it.
class LauncherChip extends StatelessWidget {
  const LauncherChip({
    super.key,
    required this.label,
    required this.state,
    required this.onTap,
  });

  final String label;
  final LauncherChipState state;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final on = state == LauncherChipState.on;
    final color = switch (state) {
      LauncherChipState.on => t.accent,
      LauncherChipState.off => t.text2,
      LauncherChipState.unavailable => t.text3,
    };
    return Semantics(
      button: true,
      label: label,
      value: switch (state) {
        LauncherChipState.on => 'on',
        LauncherChipState.off => 'off',
        LauncherChipState.unavailable => 'not for this agent',
      },
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: on ? t.accentSoft : Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(t.rControl),
          side: BorderSide(color: on ? t.accent : t.border, width: t.bw),
        ),
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(t.rControl),
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.375,
              vertical: t.sp * 0.75,
            ),
            child: Text(
              label,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: color,
                decoration: state == LauncherChipState.unavailable
                    ? TextDecoration.lineThrough
                    : null,
                decorationColor: t.text3,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The Model field's line: suggestions to fill in, then the form the agent
/// takes, or why it has none.
class _ModelUnder extends StatelessWidget {
  const _ModelUnder({
    required this.agent,
    required this.hint,
    required this.suggestions,
    required this.onPick,
  });

  final String agent;
  final String? hint;

  /// Names that fill the field in; with them, the line says the rest of the
  /// form itself, so [hint] is not repeated.
  final List<String> suggestions;
  final ValueChanged<String> onPick;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.labelSize * 1.09,
      height: 1.45,
      color: t.text3,
    );
    if (hint == null) {
      return _Under(
        key: const Key('launcher-model-hint'),
        text: '$agent has no model',
      );
    }
    if (suggestions.isEmpty) {
      return _Under(key: const Key('launcher-model-hint'), text: hint!);
    }
    return Wrap(
      key: const Key('launcher-model-hint'),
      spacing: t.sp * 0.75,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        Text('Or pick one:', style: style),
        for (final m in suggestions)
          InkWell(
            key: Key('launcher-model-$m'),
            onTap: () => onPick(m),
            borderRadius: BorderRadius.circular(t.rControl),
            child: Text(m, style: style.copyWith(color: t.accent)),
          ),
        Text('or a full id like claude-sonnet-4-5', style: style),
      ],
    );
  }
}

/// Where's picker (153-5): each online host's workspaces under its name,
/// then New workspace….
class _WhereMenu extends StatelessWidget {
  const _WhereMenu({
    required this.hosts,
    required this.spaces,
    required this.failed,
    required this.selected,
    required this.room,
    required this.onPick,
  });

  final List<AgentHost> hosts;
  final Map<String, List<AgentWorkspace>?> spaces;
  final Set<String> failed;
  final String? selected;
  final double room;
  final ValueChanged<String> onPick;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final one = hosts.length == 1;
    return StormPopover(
      key: const Key('where-menu'),
      padding: t.sp * 0.75,
      children: [
        ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: math.min(t.sp * 52.5, room - t.sp * 3),
          ),
          child: SingleChildScrollView(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final h in hosts) ...[
                  Padding(
                    padding: EdgeInsets.fromLTRB(
                      t.sp * 0.75,
                      t.sp * 0.75,
                      t.sp * 0.75,
                      t.sp * 0.5,
                    ),
                    child: AgentsLabel(h.name),
                  ),
                  if (spaces[h.id] case final ws?) ...[
                    for (final w in ws)
                      PopoverItem(
                        key: Key('where-${h.id}-${w.name}'),
                        label: w.name,
                        subtitle: w.liveSessions > 0
                            ? 'another session is working here'
                            : null,
                        selected: _whereKey(h.id, w.name) == selected,
                        onTap: () => onPick(_whereKey(h.id, w.name)),
                      ),
                    if (ws.isEmpty) const PopoverItem(label: 'No workspaces'),
                  ] else
                    PopoverItem(
                      label: failed.contains(h.id)
                          ? "Couldn't list its workspaces"
                          : '…',
                    ),
                ],
                Divider(height: t.sp * 1.5, color: t.border),
                for (final h in hosts)
                  PopoverItem(
                    key: Key('where-new-${h.id}'),
                    label: one
                        ? 'New workspace…'
                        : 'New workspace on ${h.name}…',
                    leading: Icon(
                      LucideIcons.plus,
                      size: t.codeSize,
                      color: t.text2,
                    ),
                    onTap: () => onPick(_whereKey(h.id, _newWorkspaceKey)),
                  ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

/// New workspace…, inline under Where (153-5; decision 141c): a name and
/// whether to start it as a git repository (on by default). It stays open
/// on a refusal, saying why, and hands back the name once the host has
/// made it.
class _NewWorkspaceForm extends ConsumerStatefulWidget {
  const _NewWorkspaceForm({
    super.key,
    required this.host,
    required this.onMade,
    required this.onCancel,
  });

  final AgentHost host;
  final ValueChanged<String> onMade;
  final VoidCallback onCancel;

  @override
  ConsumerState<_NewWorkspaceForm> createState() => _NewWorkspaceFormState();
}

class _NewWorkspaceFormState extends ConsumerState<_NewWorkspaceForm> {
  late final _name = TextEditingController();
  bool _git = true;
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _create() async {
    final name = _name.text.trim();
    if (name.isEmpty || _busy) return;
    final api = agentApi(ref);
    if (api == null) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final made = await api.createWorkspace(widget.host.id, name, git: _git);
      if (mounted) widget.onMade(made);
    } catch (e) {
      if (mounted) setState(() => _error = newWorkspaceErrorText(e, name));
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text2,
    );
    return Container(
      key: const Key('launcher-new-workspace-form'),
      padding: EdgeInsets.all(t.sp * 1.25),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          TextField(
            key: const Key('new-workspace-name'),
            controller: _name,
            autofocus: true,
            autocorrect: false,
            maxLength: 128,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.text,
            ),
            decoration: InputDecoration(
              isDense: true,
              counterText: '',
              hintText: 'New workspace on ${widget.host.name}',
            ),
            onChanged: (_) => setState(() {}),
            onSubmitted: (_) => _create(),
          ),
          SizedBox(height: t.sp),
          Row(
            children: [
              Semantics(
                label: 'Start as a git repository',
                child: StormToggle(
                  key: const Key('new-workspace-git'),
                  value: _git,
                  onChanged: _busy ? null : (v) => setState(() => _git = v),
                ),
              ),
              SizedBox(width: t.sp),
              Expanded(child: Text('Start as a git repository', style: style)),
            ],
          ),
          if (_error != null) ...[
            SizedBox(height: t.sp),
            Text(
              _error!,
              key: const Key('new-workspace-error'),
              style: style.copyWith(color: t.danger),
            ),
          ],
          SizedBox(height: t.sp),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              StormButton.text(
                key: const Key('new-workspace-cancel'),
                label: 'Cancel',
                onPressed: _busy ? null : widget.onCancel,
              ),
              SizedBox(width: t.sp),
              StormButton.soft(
                key: const Key('new-workspace-create'),
                label: _busy ? 'Creating' : 'Create',
                onPressed: _busy || _name.text.trim().isEmpty ? null : _create,
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// One choice in a [_Select].
class _Opt<T> {
  const _Opt(this.value, this.label);

  final T value;
  final String label;
}

/// A select: `surface2`, a hairline, `rControl`, a trailing chevron (138h),
/// and a popover of options.
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
    this.enabled = true,
    this.alwaysOpens = false,
    this.menu,
  });

  final String label;
  final T? value;
  final List<_Opt<T>> options;
  final ValueChanged<T> onChanged;

  /// Shown in the menu, never choosable (an agent not installed).
  final List<String> disabled;
  final String placeholder;
  final bool compact;

  /// False draws it muted and opens nothing.
  final bool enabled;

  /// Opens with no options too: its [menu] offers more than them.
  final bool alwaysOpens;

  /// The popover in place of the plain list, given the height it has; it
  /// pops the value picked.
  final Widget Function(BuildContext popover, double room)? menu;

  @override
  State<_Select<T>> createState() => _SelectState<T>();
}

class _SelectState<T> extends State<_Select<T>> {
  final _anchor = GlobalKey();

  Future<void> _open() async {
    final box = _anchor.currentContext?.findRenderObject() as RenderBox?;
    final room = popoverRoom(context, _anchor);
    final picked = await showStormPopover<T>(
      context: context,
      anchorKey: _anchor,
      width: box?.size.width ?? context.tokens.sp * 30,
      builder: (popover) =>
          widget.menu?.call(popover, room) ??
          StormPopover(
            children: [
              for (final o in widget.options)
                PopoverItem(
                  label: o.label,
                  selected: o.value == widget.value,
                  onTap: () => Navigator.of(popover).pop(o.value),
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
        widget.options
            .where((o) => o.value == widget.value)
            .firstOrNull
            ?.label ??
        widget.placeholder;
    final live =
        widget.enabled && (widget.alwaysOpens || widget.options.isNotEmpty);
    return Semantics(
      button: true,
      enabled: live,
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
            child: Row(
              mainAxisSize: widget.compact
                  ? MainAxisSize.min
                  : MainAxisSize.max,
              children: [
                Flexible(
                  fit: widget.compact ? FlexFit.loose : FlexFit.tight,
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
                SizedBox(width: t.sp),
                Icon(
                  LucideIcons.chevron_down,
                  key: const Key('select-chevron'),
                  size: t.codeSize,
                  color: live ? t.text2 : t.text3,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
