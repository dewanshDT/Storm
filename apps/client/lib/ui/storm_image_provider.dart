import 'dart:ui' as ui;
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/painting.dart';
import 'package:http/http.dart' as http;

/// An [ImageProvider] that fetches image bytes through a provided [http.Client].
///
/// This is the transport-aware replacement for `Image.network`. When the
/// connection is relayed, the client is an [SrpHttpClient] and the request
/// travels through the SRP tunnel. When direct, it's a plain [http.Client].
///
/// The provider carries its own credentials via headers, so the credential
/// never appears in a URL (fixing the query-string exposure of
/// [StormApi.attachmentUrl]).
class StormImageProvider extends ImageProvider<StormImageProvider> {
  const StormImageProvider({
    required this.uri,
    required this.client,
    required this.headers,
  });

  /// The image URI (absolute or vault-relative path).
  final Uri uri;

  /// The HTTP client to use for the fetch. Injected by the caller so the
  /// transport (direct vs relayed) is transparent.
  final http.Client client;

  /// Authorization headers (and any other headers needed).
  final Map<String, String> headers;

  double get scale => 1.0;

  @override
  Future<StormImageProvider> obtainKey(ImageConfiguration configuration) {
    return SynchronousFuture<StormImageProvider>(this);
  }

  @override
  ImageStreamCompleter loadImage(
    StormImageProvider key,
    ImageDecoderCallback decode,
  ) {
    return OneFrameImageStreamCompleter(
      _loadAsync(key, decode),
      informationCollector: () sync* {
        yield ErrorDescription('Resolving image: ${key.uri}');
      },
    );
  }

  Future<ImageInfo> _loadAsync(
    StormImageProvider key,
    ImageDecoderCallback decode,
  ) async {
    final request = http.Request('GET', key.uri);
    request.headers.addAll(key.headers);

    final streamed = await key.client.send(request);

    if (streamed.statusCode != 200) {
      throw HttpException(
        'Failed to load image: ${streamed.statusCode}',
        uri: key.uri,
      );
    }

    final bytes = await streamed.stream.fold<Uint8List>(Uint8List(0), (
      acc,
      chunk,
    ) {
      final newAcc = Uint8List(acc.length + chunk.length);
      newAcc.setRange(0, acc.length, acc);
      newAcc.setRange(acc.length, newAcc.length, chunk);
      return newAcc;
    });

    final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
    final codec = await decode(buffer);
    final frame = await codec.getNextFrame();
    return ImageInfo(image: frame.image, scale: scale);
  }

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    if (other.runtimeType != runtimeType) return false;
    return other is StormImageProvider &&
        other.uri == uri &&
        other.scale == scale;
  }

  @override
  int get hashCode => Object.hash(uri, scale);

  @override
  String toString() => '$runtimeType($uri, scale: $scale)';
}

/// A convenience function to build a [StormImageProvider] from the active
/// connection.
///
/// Returns null if there's no active API (unpaired/unconfigured state).
StormImageProvider? buildStormImageProvider({
  required String vaultId,
  required String path,
  required http.Client client,
  required Map<String, String> headers,
}) {
  final url = Uri.parse(path);
  return StormImageProvider(uri: url, client: client, headers: headers);
}
