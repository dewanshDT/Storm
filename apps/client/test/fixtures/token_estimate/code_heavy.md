# Debouncing a stream in Dart

A small helper that waits for a pause before it acts. Use it for search boxes
and anything that recomputes on every keystroke.

```dart
import 'dart:async';

class Debouncer {
  Debouncer(this.delay);

  final Duration delay;
  Timer? _timer;

  void call(void Function() action) {
    _timer?.cancel();
    _timer = Timer(delay, action);
  }

  void dispose() => _timer?.cancel();
}

void main() {
  final debounce = Debouncer(const Duration(milliseconds: 300));
  for (var i = 0; i < 10; i++) {
    debounce(() => print('fired after $i'));
  }
}
```

The same idea in Rust, with tokio:

```rust
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

pub async fn debounce<T: Send + 'static>(
    mut rx: mpsc::Receiver<T>,
    delay: Duration,
    tx: mpsc::Sender<T>,
) {
    let mut pending: Option<T> = None;
    let mut deadline = Instant::now() + delay;
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(v) => { pending = Some(v); deadline = Instant::now() + delay; }
                None => break,
            },
            _ = sleep(deadline.saturating_duration_since(Instant::now())), if pending.is_some() => {
                if let Some(v) = pending.take() {
                    let _ = tx.send(v).await;
                }
            }
        }
    }
}
```

And a shell one-liner for checking the result:

```sh
for f in $(git ls-files '*.dart'); do wc -l "$f"; done | sort -n | tail -5
```

Keep the delay short (200–400 ms). Longer feels broken.
