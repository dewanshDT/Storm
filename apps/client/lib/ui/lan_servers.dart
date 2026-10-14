/// The pairing screen's "On this network" list and its pair sheet
/// (decision 118, the locked UI in the vault's *LAN Discovery Pairing*).
library;

import 'dart:async';
import 'dart:io' show Platform;

import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/auth_api.dart';
import '../api/lan_discovery.dart';
import '../api/lan_pairing.dart';
import '../api/pairing_route.dart';
import '../state/app_state.dart';
import '../state/client_version.dart';
import '../state/health.dart' show versionsCompatible;
import 'controls.dart';
import 'tokens.dart';
import 'widgets.dart';

/// Pure, so the web rule is testable: browsing never runs in a browser.
bool lanDiscoverySupportedOn({
  required bool web,
  required TargetPlatform platform,
}) =>
    !web &&
    (platform == TargetPlatform.android ||
        platform == TargetPlatform.macOS ||
        platform == TargetPlatform.linux);

final lanDiscoverySupportedProvider = Provider<bool>(
  (ref) =>
      lanDiscoverySupportedOn(web: kIsWeb, platform: defaultTargetPlatform),
);

final lanBrowserFactoryProvider = Provider<LanBrowser Function()>(
  (ref) => BonsoirLanBrowser.new,
);

/// How the sheet proves a server. Tests swap in a fake transport.
final lanRouteOpenerProvider =
    Provider<Future<LanRoute> Function(DiscoveredServer)>(
      (ref) => openLanRoute,
    );

/// The name the pair sheet suggests: the phone's model ("Pixel 10"), the
/// Mac's name, the Linux host's.
final defaultDeviceNameProvider = FutureProvider<String>((ref) async {
  try {
    final info = DeviceInfoPlugin();
    switch (defaultTargetPlatform) {
      case TargetPlatform.android:
        return (await info.androidInfo).model;
      case TargetPlatform.macOS:
        return (await info.macOsInfo).computerName;
      case TargetPlatform.iOS:
        return (await info.iosInfo).name;
      default:
        if (!kIsWeb) return Platform.localHostname;
    }
  } catch (_) {}
  return 'Storm (${defaultTargetPlatform.name})';
});

const _emptyAfter = Duration(seconds: 5);

/// "ON THIS NETWORK": browses while visible and stops when it leaves.
class LanServersSection extends ConsumerStatefulWidget {
  const LanServersSection({
    super.key,
    required this.onPair,
    required this.onScanQr,
    required this.onOpenPaired,
  });

  final void Function(DiscoveredServer server) onPair;
  final VoidCallback? onScanQr;
  final VoidCallback onOpenPaired;

  @override
  ConsumerState<LanServersSection> createState() => _LanServersSectionState();
}

class _LanServersSectionState extends ConsumerState<LanServersSection> {
  LanBrowser? _browser;
  StreamSubscription<List<DiscoveredServer>>? _sub;
  Timer? _emptyTimer;
  List<DiscoveredServer> _servers = const [];
  bool _waited = false;

  @override
  void initState() {
    super.initState();
    final browser = _browser = ref.read(lanBrowserFactoryProvider)();
    _sub = browser.start().listen(
      (servers) {
        if (mounted) setState(() => _servers = servers);
      },
      onError: (_) {
        if (mounted) setState(() => _waited = true);
      },
    );
    _emptyTimer = Timer(_emptyAfter, () {
      if (mounted) setState(() => _waited = true);
    });
  }

  @override
  void dispose() {
    _emptyTimer?.cancel();
    unawaited(_sub?.cancel());
    unawaited(_browser?.stop());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    final client = ref.watch(clientVersionProvider).value;
    final browsing = !_waited || _servers.isNotEmpty;

    return Column(
      key: const Key('lan-servers'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                'ON THIS NETWORK',
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  letterSpacing: t.labelSize * 0.08,
                  color: t.text3,
                ),
              ),
            ),
            if (browsing)
              SizedBox(
                key: const Key('lan-browsing'),
                width: t.sp * 1.5,
                height: t.sp * 1.5,
                child: const CircularProgressIndicator(strokeWidth: 1.5),
              ),
          ],
        ),
        SizedBox(height: t.sp),
        if (_servers.isEmpty && _waited)
          Text(
            key: const Key('lan-empty'),
            "No Storm servers found on this Wi-Fi. Make sure you're on the "
            "same network as your server, and that VPNs or guest networks "
            "aren't isolating devices.",
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              height: 1.45,
              color: t.text3,
            ),
          )
        else if (_servers.isNotEmpty)
          Container(
            decoration: BoxDecoration(
              color: t.surface,
              borderRadius: BorderRadius.circular(t.rCard),
              border: Border.all(color: t.border, width: t.bw),
            ),
            clipBehavior: Clip.antiAlias,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final (i, s) in _servers.indexed)
                  _LanServerRow(
                    server: s,
                    state: lanServerState(
                      s,
                      pairedServerId: settings.isPaired
                          ? settings.serverId
                          : '',
                      compatible:
                          client == null ||
                          versionsCompatible(client, s.version),
                    ),
                    divider: i > 0,
                    onPair: () => widget.onPair(s),
                    onScanQr: widget.onScanQr,
                    onOpenPaired: widget.onOpenPaired,
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

class _LanServerRow extends StatelessWidget {
  const _LanServerRow({
    required this.server,
    required this.state,
    required this.divider,
    required this.onPair,
    required this.onScanQr,
    required this.onOpenPaired,
  });

  final DiscoveredServer server;
  final LanServerState state;
  final bool divider;
  final VoidCallback onPair;
  final VoidCallback? onScanQr;
  final VoidCallback onOpenPaired;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final (detail, onTap) = switch (state) {
      LanServerState.available => (
        '${server.fingerprint} · v${server.version}',
        onPair,
      ),
      LanServerState.alreadyPaired => ('This device is paired', onOpenPaired),
      LanServerState.needsSetup => (
        'Needs setup — scan its setup QR',
        onScanQr,
      ),
      LanServerState.pairingOff => (
        'Pairing from this network is turned off',
        null,
      ),
      LanServerState.updateNeeded => (
        'Update needed — server v${server.version}',
        null,
      ),
    };
    final live = state == LanServerState.available;
    return Material(
      color: Colors.transparent,
      child: InkWell(
        key: Key('lan-server-${server.serverId}'),
        onTap: onTap,
        child: Container(
          decoration: BoxDecoration(
            border: divider
                ? Border(
                    top: BorderSide(color: t.border, width: t.bw),
                  )
                : null,
          ),
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 1.5,
            vertical: t.sp * 1.25,
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Padding(
                padding: EdgeInsets.only(top: t.sp * 0.5, right: t.sp),
                child: Container(
                  width: t.sp * 0.75,
                  height: t.sp * 0.75,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: live ? t.green : null,
                    border: live
                        ? null
                        : Border.all(color: t.text3, width: t.bw),
                  ),
                ),
              ),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            server.name,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontFamily: StormTokens.sansFamily,
                              fontSize: t.uiSize,
                              color: live ? t.text : t.text2,
                            ),
                          ),
                        ),
                        SizedBox(width: t.sp),
                        Text(
                          server.address,
                          style: TextStyle(
                            fontFamily: StormTokens.monoFamily,
                            fontSize: t.labelSize,
                            color: t.text3,
                          ),
                        ),
                      ],
                    ),
                    SizedBox(height: t.sp * 0.25),
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            detail,
                            key: Key('lan-detail-${server.serverId}'),
                            style: TextStyle(
                              fontFamily: live
                                  ? StormTokens.monoFamily
                                  : StormTokens.sansFamily,
                              fontSize: live ? t.labelSize : t.codeSize,
                              color: t.text3,
                            ),
                          ),
                        ),
                        if (onTap != null)
                          Text(
                            '›',
                            style: TextStyle(
                              fontSize: t.uiSize,
                              color: t.text3,
                            ),
                          ),
                      ],
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

enum _SheetPhase {
  verifying,
  identityFailed,
  unreachable,
  verified,
  setupRequired,
}

/// The pair sheet: identity first, and only then a password field.
class LanPairSheet extends ConsumerStatefulWidget {
  const LanPairSheet({
    super.key,
    required this.server,
    required this.onPaired,
    required this.onScanQr,
  });

  final DiscoveredServer server;

  /// Saves what pairing produced; the sheet closes after it returns.
  final Future<void> Function(LanRoute route, LanPairing paired) onPaired;
  final VoidCallback? onScanQr;

  @override
  ConsumerState<LanPairSheet> createState() => _LanPairSheetState();
}

class _LanPairSheetState extends ConsumerState<LanPairSheet> {
  final _password = TextEditingController();
  final _deviceName = TextEditingController();
  _SheetPhase _phase = _SheetPhase.verifying;
  LanRoute? _route;
  bool _pairing = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    unawaited(_verify());
    unawaited(
      ref.read(defaultDeviceNameProvider.future).then((name) {
        if (mounted && _deviceName.text.isEmpty) _deviceName.text = name;
      }, onError: (_) {}),
    );
  }

  @override
  void dispose() {
    _route?.close();
    _password.dispose();
    _deviceName.dispose();
    super.dispose();
  }

  Future<void> _verify() async {
    try {
      final route = await ref.read(lanRouteOpenerProvider)(widget.server);
      if (!mounted) {
        route.close();
        return;
      }
      setState(() {
        _route = route;
        _phase = _SheetPhase.verified;
      });
    } on PairingRouteException catch (e) {
      if (!mounted) return;
      setState(
        () => _phase = e.failure == PairingFailure.serverUnreachable
            ? _SheetPhase.unreachable
            : _SheetPhase.identityFailed,
      );
    } catch (_) {
      if (mounted) setState(() => _phase = _SheetPhase.unreachable);
    }
  }

  Future<void> _pair() async {
    final route = _route;
    final password = _password.text;
    final name = _deviceName.text.trim();
    if (route == null || password.isEmpty || name.isEmpty || _pairing) return;
    setState(() {
      _pairing = true;
      _error = null;
    });
    try {
      final paired = await pairOverLan(
        route,
        password: password,
        deviceName: name,
        platform: defaultTargetPlatform.name.toLowerCase(),
        version: ref.read(clientVersionProvider).value,
      );
      await widget.onPaired(route, paired);
      if (mounted) Navigator.of(context).pop(true);
    } on AuthApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _pairing = false;
        if (isSetupRequired(e)) {
          _phase = _SheetPhase.setupRequired;
        } else {
          _error = lanPairingError(e);
        }
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _pairing = false;
        _error = "Couldn't reach ${widget.server.name}. Try again.";
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final body = switch (_phase) {
      _SheetPhase.verifying => _status(
        t,
        key: const Key('lan-verifying'),
        spinner: true,
        text: "Checking Storm's identity…",
      ),
      _SheetPhase.identityFailed => _status(
        t,
        key: const Key('lan-identity-failed'),
        text:
            "This server didn't prove its identity. Don't enter your "
            'password.',
        danger: true,
      ),
      _SheetPhase.unreachable => _status(
        t,
        key: const Key('lan-unreachable'),
        text:
            "Couldn't reach ${widget.server.name} at "
            '${widget.server.address}. Your password was not sent.',
        danger: true,
      ),
      _SheetPhase.setupRequired => _setupRequired(t),
      _SheetPhase.verified => _verified(t),
    };
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.only(
          left: t.cardPad,
          right: t.cardPad,
          top: t.cardPad,
          bottom: MediaQuery.of(context).viewInsets.bottom + t.cardPad,
        ),
        child: SingleChildScrollView(child: body),
      ),
    );
  }

  Widget _status(
    StormTokens t, {
    required Key key,
    required String text,
    bool spinner = false,
    bool danger = false,
  }) => Column(
    key: key,
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      _title(t, widget.server.name),
      SizedBox(height: t.sp * 2),
      Row(
        children: [
          if (spinner) ...[
            SizedBox(
              width: t.sp * 2,
              height: t.sp * 2,
              child: const CircularProgressIndicator(strokeWidth: 2),
            ),
            SizedBox(width: t.sp * 1.5),
          ],
          Expanded(
            child: Text(
              text,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.uiSize,
                height: 1.45,
                color: danger ? t.danger : t.text2,
              ),
            ),
          ),
        ],
      ),
    ],
  );

  Widget _title(StormTokens t, String text) => Text(
    text,
    style: TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.titleSize,
      fontWeight: FontWeight.w600,
      color: t.text,
    ),
  );

  Widget _setupRequired(StormTokens t) => Column(
    key: const Key('lan-setup-required'),
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      _title(t, widget.server.name),
      SizedBox(height: t.sp * 2),
      Text(
        'This Storm has no account yet. Set it up by scanning its setup QR: '
        'run `storm-server pair --qr` on the server, or open the QR in its '
        'log.',
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.uiSize,
          height: 1.45,
          color: t.text2,
        ),
      ),
      if (widget.onScanQr != null) ...[
        SizedBox(height: t.sp * 2),
        StormButton.primary(
          label: 'Scan QR code',
          onPressed: () {
            Navigator.of(context).pop(false);
            widget.onScanQr!();
          },
        ),
      ],
    ],
  );

  Widget _verified(StormTokens t) => Column(
    key: const Key('lan-verified'),
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      _title(t, 'Pair with ${widget.server.name}'),
      SizedBox(height: t.sp * 2),
      Text(
        widget.server.fingerprint,
        key: const Key('lan-fingerprint'),
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.titleSize,
          letterSpacing: t.titleSize * 0.06,
          color: t.text,
        ),
      ),
      SizedBox(height: t.sp * 1.5),
      Text(
        'If this is your first device, this is it. Otherwise check it '
        'matches Settings › Connection › Server identity on a paired device, '
        'or `storm-server status`. Scanning a QR from a paired device is the '
        'safest way.',
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.codeSize,
          height: 1.45,
          color: t.text3,
        ),
      ),
      SizedBox(height: t.sp * 2.5),
      StormInput(
        key: const Key('lan-password'),
        controller: _password,
        autofocus: true,
        labelText: 'Password',
        obscureText: true,
        autocorrect: false,
        onSubmitted: (_) => _pair(),
      ),
      SizedBox(height: t.sp * 1.5),
      StormInput(
        key: const Key('lan-device-name'),
        controller: _deviceName,
        labelText: 'Device name',
        autocorrect: false,
      ),
      if (_error != null) ...[
        SizedBox(height: t.sp * 1.5),
        Text(
          _error!,
          key: const Key('lan-error'),
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.danger,
          ),
        ),
      ],
      SizedBox(height: t.sp * 2.5),
      StormButton.primary(
        key: const Key('lan-pair'),
        label: _pairing ? 'Pairing…' : 'Pair',
        onPressed: _pairing ? null : _pair,
        expand: true,
      ),
    ],
  );
}
