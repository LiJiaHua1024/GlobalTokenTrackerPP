import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/theme.dart';

void main() {
  test('currency conversion uses cached rate and preserves original values',
      () {
    final p = ThemeProvider();
    expect(p.formatMoney(10), r'$10.00');
    p.applyCurrencyStatus(
        {'display': 'CNY', 'usd_to': 7.2, 'source': 'fixture', 'stale': true});
    expect(p.formatMoney(10), '¥72.00');
    expect(p.exchangeStale, isTrue);
    p.applyCurrencyStatus({'display': 'CNY', 'usd_to': 1.0});
    expect(p.displayCurrency, 'USD');
    expect(p.formatMoney(10), r'$10.00');
    expect(p.requestedCurrency, 'CNY');
    p.dispose();
  });
}
