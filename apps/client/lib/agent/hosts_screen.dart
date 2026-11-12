import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import '../ui/clipboard_copy.dart';
import '../ui/controls.dart';
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_api.dart';

/// The machine being enrolled. It is not necessarily the one this client
/// runs on, so the enroll sheet offers every platform (AM40, AM56).
enum HostPlatform { linux, macos, windows }

/// The platform the enroll sheet opens on: the client's own when it is a Mac
/// or a Windows machine (native or web), Linux everywhere else.
HostPlatform defaultHostPlatform() => switch (defaultTargetPlatform) {
  TargetPlatform.macOS => HostPlatform.macos,
  TargetPlatform.windows => HostPlatform.windows,
  _ => HostPlatform.linux,
};

/// One command the operator runs on the host, in order.
class EnrollStep {
  const EnrollStep(this.command, {this.pastesString = false});

  final String command;

  /// The step that asks for the enrollment string.
  final bool pastesString;
}

/// How the host runs: as the person who installs it, with their own CLIs and
/// logins (decision 151), or as its own account, a system service. The sheet
/// lists them in this order: as you is the default for a new host (151 E5).
enum EnrollShape { you, service }

/// What to run on a host of [platform] for [shape], and nothing of the
/// others'. As you: the installer on Linux (`install --user`, systemd
/// --user; there is no fixed tarball URL), `install --user` from Homebrew's
/// binary on macOS (a LaunchAgent) and from the release zip on Windows (a
/// logon task). As its own account: the `.deb` and systemd (decision 77e),
/// Homebrew and a launchd LaunchDaemon (D14, AM35), the release zip and a
/// Windows Service (D16, AM50).
List<EnrollStep> enrollSteps(
  HostPlatform platform,
  EnrollShape shape,
) => switch ((platform, shape)) {
  (HostPlatform.linux, EnrollShape.you) => const [
    EnrollStep(
      'curl -fsSL https://dewanshdt.github.io/Storm/install.sh | '
      'sh -s -- --yes --runtime --user',
    ),
    EnrollStep('~/.local/bin/storm-runtime enroll', pastesString: true),
    EnrollStep('systemctl --user enable --now storm-runtime'),
  ],
  (HostPlatform.linux, EnrollShape.service) => const [
    EnrollStep('sudo apt install storm-runtime'),
    EnrollStep(
      'sudo -u storm-runtime storm-runtime enroll',
      pastesString: true,
    ),
    EnrollStep('sudo systemctl enable --now storm-runtime'),
  ],
  (HostPlatform.macos, EnrollShape.you) => const [
    EnrollStep('brew tap dewanshdt/storm https://github.com/dewanshDT/Storm'),
    EnrollStep('brew install storm-runtime'),
    EnrollStep('"\$(brew --prefix)/bin/storm-runtime" install --user'),
    EnrollStep(
      '"\$HOME/Library/Application Support/StormRuntime/bin/storm-runtime" '
      'enroll',
      pastesString: true,
    ),
  ],
  (HostPlatform.macos, EnrollShape.service) => const [
    EnrollStep('brew tap dewanshdt/storm https://github.com/dewanshDT/Storm'),
    EnrollStep('brew install storm-runtime'),
    // The full path: `sudo` need not search Homebrew's prefix.
    EnrollStep('sudo "\$(brew --prefix)/bin/storm-runtime" install'),
    // `cd /` first: the hidden account cannot read the operator's current
    // directory, and the command fails before asking for the string (B-4).
    EnrollStep(
      'cd / && sudo -u _stormruntime /Library/StormRuntime/bin/storm-runtime enroll',
      pastesString: true,
    ),
  ],
  // In a PowerShell that is not elevated. `install --user` copies the binary
  // into %LOCALAPPDATA%; enrolling starts the logon task.
  (HostPlatform.windows, EnrollShape.you) => const [
    EnrollStep(_windowsDownload),
    EnrollStep(
      '& "\$env:TEMP\\storm-runtime\\storm-runtime.exe" install --user',
    ),
    EnrollStep(
      '& "\$env:LOCALAPPDATA\\StormRuntime\\bin\\storm-runtime.exe" enroll',
      pastesString: true,
    ),
  ],
  // In an elevated PowerShell. `install` copies the binary into Program
  // Files, so `enroll` runs from there; enrolling starts the service.
  (HostPlatform.windows, EnrollShape.service) => const [
    EnrollStep(_windowsDownload),
    EnrollStep('& "\$env:TEMP\\storm-runtime\\storm-runtime.exe" install'),
    EnrollStep(
      '& "\$env:ProgramFiles\\StormRuntime\\storm-runtime.exe" enroll',
      pastesString: true,
    ),
  ],
};

const _windowsDownload =
    'Invoke-WebRequest https://github.com/dewanshDT/Storm/releases/latest/download/'
    'storm-runtime-windows-x86_64.zip -OutFile \$env:TEMP\\storm-runtime.zip; '
    'Expand-Archive -Force \$env:TEMP\\storm-runtime.zip \$env:TEMP\\storm-runtime';

/// A shape's heading and what it means, in a line.
(String, String) enrollShapeTitle(EnrollShape shape) => switch (shape) {
  EnrollShape.you => (
    'As you',
    'Your own CLIs and logins, no administrator. The default.',
  ),
  EnrollShape.service => (
    'As its own account',
    'A system service with its own files, set up as administrator.',
  ),
};

/// Where the host runs and keeps its workspaces, in a line.
String enrollFootnote(HostPlatform platform, EnrollShape shape) => switch ((
  platform,
  shape,
)) {
  (HostPlatform.linux, EnrollShape.you) =>
    'It runs as you under your systemd user manager. The installer turns '
        'on linger so it starts at boot, or prints the one command an '
        'administrator runs for that. Workspaces are the folders in '
        '~/.local/state/storm-runtime/workspaces.',
  (HostPlatform.linux, EnrollShape.service) =>
    'It runs under systemd as the storm-runtime account. Workspaces are the '
        'folders in /var/lib/storm-runtime/workspaces.',
  (HostPlatform.macos, EnrollShape.you) =>
    'launchd starts it once it is enrolled (the LaunchAgent '
        'dev.storm.runtime), as you, while you are logged in. Workspaces '
        'are the folders in '
        '~/Library/Application Support/StormRuntime/state/workspaces.',
  (HostPlatform.macos, EnrollShape.service) =>
    'launchd starts it by itself once it is enrolled (the LaunchDaemon '
        'dev.storm.runtime), as the dedicated _stormruntime account. '
        'Workspaces are the folders in /Library/StormRuntime/workspaces.',
  (HostPlatform.windows, EnrollShape.you) =>
    'Run them in PowerShell, not as administrator. Enrolling starts a '
        'logon task that runs it as you while you are logged on. Workspaces '
        'are the folders in %LOCALAPPDATA%\\StormRuntime\\state\\workspaces.',
  (HostPlatform.windows, EnrollShape.service) =>
    'Run them in PowerShell as administrator. Enrolling starts the '
        'StormRuntime service, as the virtual account NT SERVICE\\StormRuntime. '
        'Workspaces are the folders in C:\\ProgramData\\StormRuntime\\workspaces.',
};

/// The enrollment string, shown once: it carries a single-use secret that
/// nothing stores.
class EnrollDialog extends StatefulWidget {
  const EnrollDialog({
    super.key,
    required this.enrollment,
    required this.expires,
    this.title = 'Enroll a host',
  });

  final String enrollment;
  final String expires;

  /// "Re-enroll build-vm" for a revoked host (decision 153).
  final String title;

  @override
  State<EnrollDialog> createState() => _EnrollDialogState();
}

/// The enrollment string with its secret hidden: the scheme, then the last
/// four characters, enough to tell two strings apart (B-6). The sheet ends up
/// in screenshots; the string works once, but there is no need to show it.
String maskEnrollment(String s) {
  const shown = 'storm-enroll:v1:';
  if (s.length <= shown.length + 4) return '•' * s.length;
  final head = s.startsWith(shown) ? shown : '';
  return '$head${'•' * 12}${s.substring(s.length - 4)}';
}

class _EnrollDialogState extends State<EnrollDialog> {
  bool _copied = false;
  bool _revealed = false;
  HostPlatform _platform = defaultHostPlatform();

  /// The command whose copy button last succeeded, as `<shape>-<index>`.
  String? _copiedCommand;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final until = DateTime.tryParse(widget.expires)?.toLocal();
    final when = until == null
        ? 'soon'
        : '${until.hour.toString().padLeft(2, '0')}:'
              '${until.minute.toString().padLeft(2, '0')}';
    final mono = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
    );
    final box = BoxDecoration(
      color: t.surface2,
      borderRadius: BorderRadius.circular(t.rControl),
    );
    return AlertDialog(
      title: Text(widget.title),
      content: SizedBox(
        width: t.sp * 65,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              ChoiceChips<HostPlatform>(
                key: const Key('enroll-platform'),
                options: const [
                  ChoiceOption(HostPlatform.linux, 'Linux'),
                  ChoiceOption(HostPlatform.macos, 'macOS'),
                  ChoiceOption(HostPlatform.windows, 'Windows'),
                ],
                selected: _platform,
                onSelected: (p) => setState(() {
                  _platform = p;
                  _copiedCommand = null;
                }),
              ),
              SizedBox(height: t.sp * 1.5),
              Text(
                'On the host, run one of these sets in order, and paste this '
                'string when enroll asks for it. It works once, until $when.',
              ),
              SizedBox(height: t.sp),
              Container(
                padding: EdgeInsets.only(left: t.sp),
                decoration: box,
                child: Row(
                  children: [
                    Expanded(
                      child: SelectableText(
                        _revealed
                            ? widget.enrollment
                            : maskEnrollment(widget.enrollment),
                        key: const Key('enrollment-string'),
                        style: mono,
                      ),
                    ),
                    IconButton(
                      key: const Key('reveal-enrollment'),
                      tooltip: _revealed ? 'Hide' : 'Show',
                      iconSize: t.sp * 2,
                      icon: Icon(
                        _revealed ? LucideIcons.eye_off : LucideIcons.eye,
                      ),
                      onPressed: () => setState(() => _revealed = !_revealed),
                    ),
                  ],
                ),
              ),
              for (final shape in EnrollShape.values) ...[
                SizedBox(height: t.sp * 1.5),
                Text(
                  enrollShapeTitle(shape).$1,
                  key: Key('enroll-shape-${shape.name}'),
                  style: const TextStyle(fontWeight: FontWeight.w600),
                ),
                Text(
                  enrollShapeTitle(shape).$2,
                  style: TextStyle(fontSize: t.labelSize, color: t.text3),
                ),
                SizedBox(height: t.sp * 0.5),
                for (final (i, step) in enrollSteps(
                  _platform,
                  shape,
                ).indexed) ...[
                  Container(
                    key: Key('enroll-${shape.name}-command-$i'),
                    padding: EdgeInsets.only(left: t.sp),
                    decoration: box,
                    child: Row(
                      children: [
                        Expanded(
                          child: SelectableText(step.command, style: mono),
                        ),
                        IconButton(
                          key: Key('copy-${shape.name}-command-$i'),
                          tooltip: 'Copy',
                          iconSize: t.sp * 2,
                          icon: Icon(
                            _copiedCommand == '${shape.name}-$i'
                                ? LucideIcons.check
                                : LucideIcons.copy,
                          ),
                          onPressed: () async {
                            final ok = await copyToClipboard(step.command);
                            if (mounted) {
                              setState(
                                () => _copiedCommand = ok
                                    ? '${shape.name}-$i'
                                    : null,
                              );
                            }
                          },
                        ),
                      ],
                    ),
                  ),
                  SizedBox(height: t.sp * 0.5),
                ],
                Text(
                  enrollFootnote(_platform, shape),
                  key: Key('enroll-footnote-${shape.name}'),
                  style: TextStyle(fontSize: t.labelSize, color: t.text3),
                ),
              ],
              SizedBox(height: t.sp * 0.5),
              Text(
                'Close this and the string is gone. Issue a new one if you need '
                'it.',
                style: TextStyle(fontSize: t.labelSize, color: t.text3),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton.icon(
          onPressed: () async {
            final ok = await copyToClipboard(widget.enrollment);
            if (mounted) setState(() => _copied = ok);
          },
          icon: Icon(
            _copied ? LucideIcons.check : LucideIcons.copy,
            size: t.sp * 2,
          ),
          label: Text(_copied ? 'Copied' : 'Copy string'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Done'),
        ),
      ],
    );
  }
}

/// Why a change to a host's Limits was refused, in words.
String hostLimitsRefusal(Object e) => switch (e is AgentRefusal
    ? e.code
    : null) {
  'limit_out_of_range' =>
    'Agents per session is 1 to 10, agents per tree 1 to 24, and concurrent '
        'sessions at least 1.',
  'tree_below_session' =>
    'Agents per tree must be at least agents per session.',
  'owner_only' => 'Only the host’s owner can change its limits.',
  _ => describeFailure(e),
};
