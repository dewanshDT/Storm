/// "Report a problem" (decision 162a): one sheet, reached from ⌘K, Settings
/// › About & health, and a "Report this" snackbar action. Built against
/// *Decision 162 — feedback API contract* — 162-3 (the server side) lands in
/// parallel, so this talks to [FeedbackApi] and nothing else.
library;

import 'dart:async' show unawaited;
import 'dart:convert' show JsonEncoder, base64Encode;
import 'dart:typed_data' show Uint8List;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../agent/desktop_platform.dart' show currentPlatformName;
import '../api/connection_log.dart' show ConnectionLog;
import '../api/feedback_api.dart';
import '../api/feedback_models.dart';
import '../state/app_state.dart' show rootMessengerKey;
import '../state/client_version.dart' show clientVersionProvider;
import '../state/feedback_state.dart';
import 'breakpoints.dart';
import 'clipboard_copy.dart';
import 'controls.dart';
import 'feedback_screenshot.dart';
import 'settings/settings_widgets.dart' show InfoBox, TextAction;
import 'states.dart' show describeFailure;
import 'surfaces.dart';
import 'tokens.dart';
import 'widgets.dart';

/// Opens the report sheet: a dialog at desk width, a bottom sheet on a
/// phone, mirroring how every other sheet here already splits.
///
/// Captures the screenshot **before** the sheet is drawn — the whole point
/// of taking one at all is a picture of what the person was looking at, not
/// of the report form itself.
Future<void> showReportProblemSheet(
  BuildContext context, {
  FeedbackErrorContext? error,
}) async {
  final screenshot = await captureAppScreenshot(
    pixelRatio: MediaQuery.devicePixelRatioOf(context),
  );
  if (!context.mounted) return;

  const title = 'Report a problem';
  if (context.isExpanded) {
    final t = context.tokens;
    await showDialog<void>(
      context: context,
      builder: (dialogContext) => Dialog(
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(t.rCard),
        ),
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxWidth: 480,
            maxHeight: MediaQuery.sizeOf(dialogContext).height * 0.85,
          ),
          child: StormSheet(
            title: title,
            onClose: () => Navigator.of(dialogContext).pop(),
            child: ReportProblemSheet(screenshot: screenshot, error: error),
          ),
        ),
      ),
    );
  } else {
    await showStormSheet<void>(
      context: context,
      title: title,
      heightFactor: 0.92,
      builder: (_) => ReportProblemSheet(screenshot: screenshot, error: error),
    );
  }
}

const _kindOptions = [
  ChoiceOption('bug', 'Bug'),
  ChoiceOption('idea', 'Idea'),
  ChoiceOption('other', 'Other'),
];

/// The sheet's own content — a dialog's body or a bottom sheet's, the
/// caller decides which. Public so a test can pump it directly.
class ReportProblemSheet extends ConsumerStatefulWidget {
  const ReportProblemSheet({super.key, this.screenshot, this.error});

  /// Captured at the moment the sheet was asked for; `null` means capture
  /// failed (routinely, on the web) or there was nothing to capture.
  final Uint8List? screenshot;

  /// The failing action, prefilled from a "Report this" snackbar action.
  final FeedbackErrorContext? error;

  @override
  ConsumerState<ReportProblemSheet> createState() => _ReportProblemSheetState();
}

class _ReportProblemSheetState extends ConsumerState<ReportProblemSheet> {
  final _textController = TextEditingController();
  final _emailController = TextEditingController();

  String _kind = 'bug';
  Uint8List? _screenshot;
  bool _includeDiagnostics = true;
  bool _busy = false;
  String? _error;
  FeedbackPreview? _preview;
  bool _previewStale = true;
  FeedbackStatus? _status;
  bool _statusLoading = true;

  @override
  void initState() {
    super.initState();
    _screenshot = widget.screenshot;
    unawaited(_loadStatus());
  }

  @override
  void dispose() {
    _textController.dispose();
    _emailController.dispose();
    super.dispose();
  }

  Future<void> _loadStatus() async {
    final api = feedbackApi(ref);
    if (api == null) {
      if (mounted) {
        setState(() {
          _statusLoading = false;
          _error = 'Sign in first.';
        });
      }
      return;
    }
    try {
      final status = await api.status();
      if (!mounted) return;
      setState(() {
        _status = status;
        _statusLoading = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _statusLoading = false;
        _error = _feedbackFailureMessage(e);
      });
    } finally {
      api.dispose();
    }
  }

  FeedbackClientInfo _buildClientInfo() {
    final trail = ref.read(actionTrailProvider);
    final version = ref.read(clientVersionProvider).value ?? '?';
    final platform = currentPlatformName();
    final kind = switch (platform) {
      'android' => 'android',
      'ios' => 'ios',
      'web' => 'web',
      _ => 'desktop',
    };
    final log = ConnectionLog.instance.entries;
    final tail = log.length > 200 ? log.sublist(log.length - 200) : log;
    return FeedbackClientInfo(
      version: version,
      platform: platform,
      clientKind: kind,
      screen: trail.screen,
      actions: trail.actions,
      error: widget.error,
      connectionLog: [for (final e in tail) e.toString()],
    );
  }

  /// A fresh preview, reusing the last one unless the kind or the
  /// diagnostics toggle changed since it was taken.
  Future<FeedbackPreview?> _ensurePreview() async {
    if (_preview != null && !_previewStale) return _preview;
    final api = feedbackApi(ref);
    if (api == null) {
      setState(() => _error = 'Sign in first.');
      return null;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final preview = await api.preview(
        kind: _kind,
        includeDiagnostics: _includeDiagnostics,
        client: _buildClientInfo(),
      );
      if (mounted) {
        setState(() {
          _preview = preview;
          _previewStale = false;
        });
      }
      return preview;
    } catch (e) {
      if (mounted) setState(() => _error = _feedbackFailureMessage(e));
      return null;
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _seeWhatsSent() async {
    final preview = await _ensurePreview();
    if (preview == null || !mounted) return;
    final pretty = const JsonEncoder.withIndent(
      '  ',
    ).convert({'client': preview.client, 'bundle': preview.bundle});
    await showStormSheet<void>(
      context: context,
      title: "What's sent",
      heightFactor: 0.8,
      builder: (_) => _JsonView(json: pretty),
    );
  }

  void _setKind(String kind) {
    if (kind == _kind) return;
    setState(() {
      _kind = kind;
      _previewStale = true;
    });
  }

  void _setIncludeDiagnostics(bool value) {
    setState(() {
      _includeDiagnostics = value;
      _previewStale = true;
    });
  }

  Future<void> _send() async {
    final text = _textController.text.trim();
    if (text.isEmpty) {
      setState(() => _error = 'Say what happened.');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    final preview = await _ensurePreview();
    if (preview == null) {
      if (mounted) setState(() => _busy = false);
      return;
    }
    final api = feedbackApi(ref);
    if (api == null) {
      setState(() {
        _busy = false;
        _error = 'Sign in first.';
      });
      return;
    }
    // `_ensurePreview` already cleared `_busy` in its own `finally` — set it
    // again so Send stays disabled for the network round trip that follows,
    // rather than re-enabling for the gap between the two calls.
    if (mounted) setState(() => _busy = true);
    try {
      final email = _emailController.text.trim();
      final result = await api.send(
        previewId: preview.previewId,
        kind: _kind,
        text: text,
        email: email.isEmpty ? null : email,
        screenshotPngB64: _screenshot == null
            ? null
            : base64Encode(_screenshot!),
      );
      if (!mounted) return;
      Navigator.of(context).pop();
      final id = result.reportId;
      rootMessengerKey.currentState?.showSnackBar(
        SnackBar(
          content: Text(
            'Report sent · ${id.length > 8 ? id.substring(0, 8) : id}',
          ),
        ),
      );
    } catch (e) {
      if (!mounted) return;
      // A preview kept 10 minutes (162b) can expire mid-edit. Dropping it
      // means the next Send takes a fresh one rather than retrying the
      // same dead id forever.
      if (e is FeedbackApiException && e.isPreviewExpired) {
        setState(() {
          _preview = null;
          _previewStale = true;
          _busy = false;
          _error = 'That preview expired. Press Send again.';
        });
        return;
      }
      setState(() {
        _busy = false;
        _error = _feedbackFailureMessage(e);
      });
    } finally {
      api.dispose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    if (_statusLoading) {
      return const Padding(
        padding: EdgeInsets.symmetric(vertical: 32),
        child: Center(child: CircularProgressIndicator()),
      );
    }
    final disabled = _status != null && !_status!.enabled;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (disabled) ...[
          const InfoBox(
            "Reporting is turned off on this server. Ask whoever runs it to "
            'turn feedback back on.',
          ),
          SizedBox(height: t.gap),
        ],
        StormInput(
          key: const Key('report-text'),
          controller: _textController,
          hintText: 'What happened?',
          minLines: 4,
          maxLines: 8,
          autofocus: !disabled,
          enabled: !disabled,
          textCapitalization: TextCapitalization.sentences,
        ),
        SizedBox(height: t.gap),
        ChoiceChips<String>(
          key: const Key('report-kind'),
          options: _kindOptions,
          selected: _kind,
          onSelected: disabled ? null : _setKind,
        ),
        if (_screenshot != null) ...[
          SizedBox(height: t.gap),
          Align(
            alignment: Alignment.centerLeft,
            child: ImageThumbnail(
              key: const Key('report-screenshot'),
              image: MemoryImage(_screenshot!),
              onRemove: () => setState(() => _screenshot = null),
            ),
          ),
        ],
        SizedBox(height: t.gap),
        StormInput(
          key: const Key('report-email'),
          controller: _emailController,
          hintText: 'Reply email (optional)',
          keyboardType: TextInputType.emailAddress,
          enabled: !disabled,
        ),
        SizedBox(height: t.gap),
        Row(
          children: [
            Expanded(
              child: InkWell(
                onTap: disabled
                    ? null
                    : () => _setIncludeDiagnostics(!_includeDiagnostics),
                borderRadius: BorderRadius.circular(t.rControl * 0.6),
                child: Padding(
                  padding: EdgeInsets.symmetric(vertical: t.sp * 0.5),
                  child: Row(
                    children: [
                      StormCheckbox(
                        key: const Key('report-diagnostics'),
                        value: _includeDiagnostics,
                        onChanged: disabled ? null : _setIncludeDiagnostics,
                      ),
                      SizedBox(width: t.sp),
                      Expanded(
                        child: Text(
                          'Include diagnostics',
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontFamily: StormTokens.sansFamily,
                            fontSize: t.uiSize,
                            color: t.text,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
            TextAction(
              key: const Key('report-see-sent'),
              label: "See what's sent",
              accent: true,
              onTap: disabled ? null : _seeWhatsSent,
            ),
          ],
        ),
        if (_error != null) ...[
          SizedBox(height: t.gap),
          Text(
            _error!,
            key: const Key('report-error'),
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.danger,
            ),
          ),
        ],
        SizedBox(height: t.gap * 1.5),
        Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            StormButton.primary(
              key: const Key('report-send'),
              label: _busy ? 'Sending…' : 'Send',
              onPressed: disabled || _busy ? null : _send,
            ),
          ],
        ),
      ],
    );
  }
}

/// Pretty, monospace, scrollable and copyable — exactly what the fields say
/// they hold, never re-summarised.
class _JsonView extends StatelessWidget {
  const _JsonView({required this.json});

  final String json;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Align(
          alignment: Alignment.centerRight,
          child: TextAction(
            key: const Key('report-sent-copy'),
            label: 'Copy',
            accent: true,
            onTap: () => copyToClipboard(json).then((copied) {
              if (!context.mounted) return;
              ScaffoldMessenger.of(context).showSnackBar(
                SnackBar(content: Text(copied ? 'Copied' : "Couldn't copy")),
              );
            }),
          ),
        ),
        SizedBox(height: t.sp),
        Container(
          padding: EdgeInsets.all(t.sp * 1.5),
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.rControl),
            border: Border.all(color: t.border, width: t.bw),
          ),
          child: SingleChildScrollView(
            child: SelectableText(
              json,
              key: const Key('report-sent-json'),
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize * 1.09,
                color: t.text,
              ),
            ),
          ),
        ),
      ],
    );
  }
}

String _feedbackFailureMessage(Object e) {
  if (e is FeedbackApiException) {
    return switch (e.message) {
      'feedback_disabled' => 'Reporting is turned off on this server.',
      'preview_expired' => 'That preview expired. Press Send again.',
      _ => describeFailure(e),
    };
  }
  return describeFailure(e);
}
