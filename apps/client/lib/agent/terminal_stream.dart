/// Opens a session's terminal stream (decision 77d).
///
/// Native: a streamed HTTP GET carrying the bearer header. Web: `EventSource`
/// with a single-use ticket, because a browser cannot put a header on it and
/// `package:http`'s browser client buffers whole responses. Both resume by
/// `?offset=`, never `Last-Event-ID`, and both end when the server closes the
/// stream, which it does once the session has ended.
library;

export 'terminal_stream_io.dart'
    if (dart.library.js_interop) 'terminal_stream_web.dart';
