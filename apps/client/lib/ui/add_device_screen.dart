import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:qr_flutter/qr_flutter.dart';

import '../api/auth_models.dart';
import '../api/storm_api.dart';
import '../state/app_state.dart';
import 'tokens.dart';

/// Shows a pairing QR so another device can join without anyone typing a URI.
///
/// This is the half of [PairingScreen] that runs on the machine you are
/// *already* signed in on. `POST /v1/pairings` is session tier, so being here
/// is the vouching: a stranger on the LAN cannot mint one of these.
///
/// The URI is shown under the QR on purpose. Scanning is the point, but a
/// camera can be broken, refused, or absent (the desktop and web builds have
/// none), and the paste field on the other device never stops working.
class AddDeviceScreen extends ConsumerStatefulWidget {
  const AddDeviceScreen({super.key});

  @override
  ConsumerState<AddDeviceScreen> createState() => _AddDeviceScreenState();
}

class _AddDeviceScreenState extends ConsumerState<AddDeviceScreen> {
  PairingInvite? _invite;
  String? _error;
  bool _loading = false;

  @override
  void initState() {
    super.initState();
    _mint();
  }

  Future<void> _mint() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    final settings = ref.read(settingsProvider).value;
    if (settings == null || !settings.hasSession) {
      setState(() {
        _error = 'Sign in first — only a signed-in device can add another.';
        _loading = false;
      });
      return;
    }
    final api = StormApi(
      baseUrl: settings.baseUrl,
      token: settings.accessToken,
    );
    try {
      final invite = await api.issuePairing();
      if (!mounted) return;
      setState(() {
        _invite = invite;
        _loading = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = '$e';
        _loading = false;
      });
    } finally {
      api.dispose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final invite = _invite;
    return Scaffold(
      appBar: AppBar(title: const Text('Add a device')),
      body: Center(
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(24),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                if (_loading) const CircularProgressIndicator(),
                if (_error != null)
                  Text(_error!, textAlign: TextAlign.center)
                else if (invite != null) ...[
                  const Text(
                    'Scan this with Storm on the new device.',
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 20),
                  // A white plate under the code regardless of theme: a QR is
                  // read by contrast, and a dark-on-dark one does not scan.
                  Container(
                    padding: const EdgeInsets.all(16),
                    color: context.tokens.codePlate,
                    child: QrImageView(
                      data: invite.toUri(),
                      version: QrVersions.auto,
                      size: 260,
                      backgroundColor: context.tokens.codePlate,
                      eyeStyle: QrEyeStyle(
                        eyeShape: QrEyeShape.square,
                        color: context.tokens.onCodePlate,
                      ),
                      dataModuleStyle: QrDataModuleStyle(
                        dataModuleShape: QrDataModuleShape.square,
                        color: context.tokens.onCodePlate,
                      ),
                      // The URI is ~200 characters, which is a dense code —
                      // the highest correction level would push it denser
                      // still and make it harder to read, not easier.
                      errorCorrectionLevel: QrErrorCorrectLevel.L,
                    ),
                  ),
                  const SizedBox(height: 16),
                  Text(
                    'Expires ${invite.expires}',
                    style: Theme.of(context).textTheme.bodySmall,
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 8),
                  const Text(
                    'Single use. Mint a new one for each device.',
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 20),
                  SelectableText(
                    invite.toUri(),
                    style: Theme.of(context).textTheme.bodySmall,
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 12),
                  Wrap(
                    spacing: 12,
                    alignment: WrapAlignment.center,
                    children: [
                      OutlinedButton(
                        onPressed: () async {
                          await Clipboard.setData(
                            ClipboardData(text: invite.toUri()),
                          );
                          if (context.mounted) {
                            ScaffoldMessenger.of(context).showSnackBar(
                              const SnackBar(content: Text('URI copied')),
                            );
                          }
                        },
                        child: const Text('Copy URI'),
                      ),
                      FilledButton(
                        onPressed: _loading ? null : _mint,
                        child: const Text('New code'),
                      ),
                    ],
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
