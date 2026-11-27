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

## Notes

- Fonts are re-declared in `pubspec.yaml` from `../client/assets/fonts/`: a
  dependency's fonts do not reach `TextStyle(fontFamily: ...)`.
- `pubspec.lock` was seeded from the client's so both resolve the same
  versions (`flutter_lucide` removes icon names across minor releases). Copy
  the client's lock over it and run `flutter pub get` after the client's
  dependencies change.
- The generator's telemetry builder is switched off in `build.yaml`.
