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
make widgetbook-usage    # regenerate lib/usage.g.dart
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
files has no use case. "Shared" is `sharedFiles` in `tool/ui_usage.dart`, the same list the usage
index scans (`controls.dart`, `widgets.dart`, `surfaces.dart`, `panels.dart`,
`states.dart` and a few other building-block files); screens and one-feature
widgets are not in it. To exempt a widget that cannot render on its own, add it to the
`_allowed` map with the reason; the test also fails when an entry goes stale
(the widget is gone, or now has a use case). A new shared file goes in
`sharedFiles`.

## Usage index

`tool/ui_usage.dart` scans `apps/client/lib` and writes `lib/usage.g.dart`:
where each public widget of the shared files (`sharedFiles`, the coverage gate's list too) is instantiated, and
the one-off candidates, which are the `InkWell`, `GestureDetector` and
`Container`/`DecoratedBox` with a `BoxDecoration` written outside those files.
The catalogue's `[Index]` folder renders both: a table of shared components
with their use counts (zero highlighted) and the candidates grouped by file,
which is the backlog of hand-rolled controls to move onto shared components.
The defining file and generated files are not counted.

It is a text scan that skips comments and strings, not an analyzer, so a
component built through a tear-off or a helper is not seen. Run
`make widgetbook-usage` after changing the client and commit the result;
`make test-widgetbook` and CI regenerate it and fail on a diff.

## Golden tests

`test/goldens_test.dart` renders every component's use cases (through the same
builders the catalogue uses, knobs at their defaults) under Storm dark, Storm
light and SlowFlow earth, at 390 (phone) and 1280 (desktop) wide, into one
image per component in `test/goldens/ci/`. `flutter test` (so `make
test-widgetbook` and CI) compares them; after an intended visual change run

```sh
make goldens             # flutter test --update-goldens --tags golden
```

and commit the changed images. Only Alchemist's CI goldens are committed:
text is drawn as solid blocks by design, so they hold layout, colour and
spacing but not glyphs, and they render the same on every machine. Platform
goldens are switched off in `test/flutter_test_config.dart`. Because the
blocks are wider than real text, a row can overflow in a golden that does not
in the app; overflow is not a failure here. Animations are frozen
(`TickerMode` off). When CI fails, the diffs are in the
`widgetbook-golden-failures` artifact.

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
