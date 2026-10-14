import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/painting.dart';
import 'package:http/http.dart' as http;

import '../api/connection_client.dart';
import '../api/storm_api.dart';

/// An [ImageProvider] that fetches image bytes through a provided [http.Client].
///
/// This is the transport-aware replacement for `Image.network` (decision 63).
/// In the app the client is the session's [ConnectionClient], which sends each
/// fetch over whichever transport the connection has proven when it is made,
/// so an attachment loads through the relay off the LAN and after a reconnect
/// (decision 112).
///
/// The credential travels as a header, never in the URL.
class StormImageProvider extends ImageProvider<StormImageProvider> {
  const StormImageProvider({
    required this.uri,
    required this.client,
    required this.headers,
  });

  /// [path] in [vaultId], fetched with [api]'s client and credential. The URL
  /// is the saved address, whatever the transport, so the image cache keeps
  /// one entry per attachment across a switch between direct and relayed.
  factory StormImageProvider.attachment(
    StormApi api,
    String vaultId,
    String path,
  ) => StormImageProvider(
    uri: api.attachmentUri(vaultId, path),
    client: api.client,
    headers: {'Authorization': api.credential},
  );

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

  /// The image's bytes, as [loadImage] fetches them.
  Future<Uint8List> fetch() async {
    final request = http.Request('GET', uri);
    request.headers.addAll(headers);

    final streamed = await client.send(request);

    if (streamed.statusCode != 200) {
      unawaited(streamed.stream.listen(null).cancel());
      throw http.ClientException(
        'Failed to load image: ${streamed.statusCode}',
        uri,
      );
    }
    return streamed.stream.toBytes();
  }

  Future<ImageInfo> _loadAsync(
    StormImageProvider key,
    ImageDecoderCallback decode,
  ) async {
    final bytes = await key.fetch();
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
