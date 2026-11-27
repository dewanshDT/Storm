// Button() in a line comment is not a use
/* Button() and
   InkWell() in a block comment, /* nested */ still a comment */
Widget a() => const Button();
Widget b() => Button();
Widget c() => const Badge.small();
Widget d() => Chips<List<String>>();
const doc = 'Button() inside a string, and InkWell()';
const multi = '''
Button()
''';
const raw = r'Button() $x';
String interpolated(int n) => 'n ${Button()} $n';
Widget e() => ShareButton();
Widget f() => other.Button.named();
final g = Button;

Widget tap() => InkWell(onTap: () {}, child: Text('x'));
Widget detect() => GestureDetector(
  onTap: () {},
  child: InkWell(child: Container(color: red)),
);
Widget box() => Container(
  decoration: BoxDecoration(color: red),
  child: Container(padding: pad),
);
Widget outer() => Container(
  child: DecoratedBox(decoration: BoxDecoration(color: red)),
);
Widget plain() => Container(color: red, child: Text('BoxDecoration('));
