# storm_widgetbook

The component catalogue for Storm's shared UI (decision 164): every widget in
the client's `lib/ui/`, rendered in isolation under the real token and theme
layer, with knobs for its inputs and a use case per state. It depends on
`apps/client` by path and imports `lib/ui/` directly; nothing here is added to
the client.

```sh
make widgetbook          # run it (linux or macOS desktop)
make widgetbook-web      # build the web catalogue into apps/widgetbook/build/web
make widgetbook-codegen  # regenerate lib/main.directories.g.dart
```

The toolbar's addons: **Theme** (every variant of every family, loaded from the
client's `assets/themes/`; a variant is already light or dark), **Viewport**
(phone, tablet, desktop) and **Text scale**.

## Adding a use case

Put an `@UseCase` function in `lib/use_cases/<component>.dart`, one per state
(default, disabled, error, empty, long text, each variant), with knobs for the
public inputs:

```dart
@widgetbook.UseCase(name: 'disabled', type: StormButton, path: '[Controls]')
Widget stormButtonDisabled(BuildContext context) =>
    StormButton(label: 'Save', onPressed: null);
```

Then run `make widgetbook-codegen` and commit the regenerated
`lib/main.directories.g.dart`. It is committed so nothing else needs
build_runner; `make test-widgetbook` (part of `make check`) fails if it is
stale. A use case that needs app state belongs on a component that takes its
state as inputs, not on a stub of the app.

## Coverage gate

`test/coverage_test.dart` fails when a public widget in the client's shared
files has no use case. "Shared" is the `_shared` list at the top of that test
(`controls.dart`, `widgets.dart`, `surfaces.dart`, `panels.dart`, `states.dart`
and a few other building-block files); screens and one-feature widgets are not
in it. To exempt a widget that cannot render on its own, add it to the
`_allowed` map with the reason; the test also fails when an entry goes stale
(the widget is gone, or now has a use case). A new shared file goes in
`_shared`.

## Notes

- Fonts are re-declared in `pubspec.yaml` from `../client/assets/fonts/`: a
  dependency's fonts do not reach `TextStyle(fontFamily: ...)`.
- `pubspec.lock` was seeded from the client's so both resolve the same
  versions (`flutter_lucide` removes icon names across minor releases). Copy
  the client's lock over it and run `flutter pub get` after the client's
  dependencies change.
- `assets/` is a link to `../client/assets`, declared in `pubspec.yaml` for the
  one image the client loads without a `package:` prefix (the brand mark).
- The generator's telemetry builder is switched off in `build.yaml`.
