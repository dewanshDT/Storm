/// A place in the client where something is written, `file` relative to
/// `apps/client/lib`.
class UsageSite {
  const UsageSite(this.file, this.line);

  final String file;
  final int line;

  @override
  String toString() => '$file:$line';
}

enum CandidateKind {
  inkWell('InkWell'),
  gestureDetector('GestureDetector'),
  boxDecoration('Container/DecoratedBox with a BoxDecoration');

  const CandidateKind(this.label);

  final String label;
}

/// A hand-rolled control or surface that a shared component could replace.
class CandidateSite extends UsageSite {
  const CandidateSite(super.file, super.line, this.kind);

  final CandidateKind kind;
}
