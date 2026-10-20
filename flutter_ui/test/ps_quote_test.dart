import 'package:flutter_test/flutter_test.dart';

import 'package:globaltokentracker_ui/core/ps_quote.dart';

void main() {
  test('single-quoted literals double apostrophes', () {
    expect(psSingleQuote(r"C:\O'Brien"), r"C:\O''Brien");
    expect(psSingleQuote(r"C:\Users\plain"), r"C:\Users\plain");
  });

  test('double-quoted literals escape quotes, dollars and backticks', () {
    expect(
      psDoubleQuote(r'C:\$temp\a"b'),
      r'C:\`$temp\a`"b',
    );
    expect(psDoubleQuote(r'C:\a`b'), r'C:\a``b');
  });
}
