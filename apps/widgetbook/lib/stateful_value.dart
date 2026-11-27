import 'package:flutter/widgets.dart';

/// A value a use case can change by interacting with it. It resets whenever the
/// [initial] knob moves, so the knob stays the source of truth.
class StatefulValue<T> extends StatefulWidget {
  const StatefulValue({
    super.key,
    required this.initial,
    required this.builder,
  });

  final T initial;
  final Widget Function(T value, ValueChanged<T> set) builder;

  @override
  State<StatefulValue<T>> createState() => _StatefulValueState<T>();
}

class _StatefulValueState<T> extends State<StatefulValue<T>> {
  late T _value = widget.initial;

  @override
  void didUpdateWidget(StatefulValue<T> old) {
    super.didUpdateWidget(old);
    if (old.initial != widget.initial) _value = widget.initial;
  }

  @override
  Widget build(BuildContext context) =>
      widget.builder(_value, (v) => setState(() => _value = v));
}
