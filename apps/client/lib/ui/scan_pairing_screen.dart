import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../api/auth_models.dart';
import 'tokens.dart';

/// Whether this build can offer a camera scanner at all.
///
/// Desktop and web have no camera path here, and offering a button that opens
/// a black rectangle is the same failure as the copy that said "scan" when
/// nothing could — so the caller hides the affordance rather than showing one
/// that fails.
bool get canScanPairingQr =>
    !kIsWeb &&
    (defaultTargetPlatform == TargetPlatform.android ||
        defaultTargetPlatform == TargetPlatform.iOS);

/// Reads a `storm://pair` QR and returns its URI, or null if dismissed.
///
/// **This is an input method and nothing more.** It hands the decoded string
/// back to the caller, which runs the same [PairingUri.parse] the paste field
/// runs — so verification, the whitespace repair and every refusal are shared.
/// A scanner that parsed on its own would be a second implementation of the
/// security-critical step, and the two would drift.
class ScanPairingScreen extends StatefulWidget {
  const ScanPairingScreen({super.key});

  @override
  State<ScanPairingScreen> createState() => _ScanPairingScreenState();
}

class _ScanPairingScreenState extends State<ScanPairingScreen> {
  final _controller = MobileScannerController(
    // One code is all we want, and the pairing URI is not a barcode.
    formats: const [BarcodeFormat.qrCode],
  );

  /// A camera fires the same code many times a second. Without this the route
  /// pops repeatedly and Navigator throws on the second one.
  bool _handled = false;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _onDetect(BarcodeCapture capture) {
    if (_handled) return;
    for (final barcode in capture.barcodes) {
      final raw = barcode.rawValue;
      if (raw == null) continue;
      // Only accept something that actually parses, so a stray QR on the desk
      // (a wifi code, a URL) does not close the scanner and send the caller
      // back with rubbish.
      if (PairingUri.parse(raw) == null) continue;
      _handled = true;
      Navigator.of(context).pop(raw);
      return;
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Scan pairing code')),
      body: Stack(
        alignment: Alignment.center,
        children: [
          MobileScanner(
            controller: _controller,
            onDetect: _onDetect,
            // A camera that cannot start is a normal outcome — permission
            // refused, or in use elsewhere. Say so and leave the paste field
            // as the way through, rather than showing a dead viewfinder.
            errorBuilder: (context, error) => Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  const Text(
                    'The camera is not available.',
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 8),
                  Text(
                    '${error.errorCode}',
                    textAlign: TextAlign.center,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                  const SizedBox(height: 16),
                  const Text(
                    'Go back and paste the pairing URI instead.',
                    textAlign: TextAlign.center,
                  ),
                ],
              ),
            ),
          ),
          IgnorePointer(
            child: Container(
              width: 240,
              height: 240,
              decoration: BoxDecoration(
                // The reticle sits over a camera feed, not over a themed
                // surface, so it takes the fixed code colours for the same
                // reason the QR plate does.
                border: Border.all(color: context.tokens.codePlate, width: 2),
                borderRadius: BorderRadius.circular(context.tokens.rCard),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
