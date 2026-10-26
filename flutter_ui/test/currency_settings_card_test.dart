import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/theme.dart';
import 'package:globaltokentracker_ui/widgets/currency_settings_card.dart';
import 'package:provider/provider.dart';

class _CurrencyProvider extends ThemeProvider {
  bool fail = false;

  @override
  Future<void> setCurrency(String code) async {
    if (fail) throw StateError('fixture failure');
    applyCurrencyStatus({'display': code, 'usd_to': 7.2, 'source': 'fixture'});
  }
}

void main() {
  for (final brightness in Brightness.values) {
    testWidgets('currency selection has a stable rounded fill in $brightness',
        (tester) async {
      final provider = _CurrencyProvider();
      addTearDown(provider.dispose);
      final theme = brightness == Brightness.light
          ? provider.lightTheme
          : provider.darkTheme;
      await tester.pumpWidget(ChangeNotifierProvider<ThemeProvider>.value(
        value: provider,
        child: MaterialApp(
          theme: theme,
          home: const Scaffold(body: CurrencySettingsCard()),
        ),
      ));
      await tester.pumpAndSettle();
      final dropdown = find.byType(DropdownButton<String>);
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pumpAndSettle();
      final decorator =
          tester.widget<InputDecorator>(find.byType(InputDecorator));
      expect(decorator.isFocused, isTrue);
      expect(decorator.decoration.fillColor,
          theme.colorScheme.surfaceContainerLow);
      expect(decorator.decoration.border, isA<OutlineInputBorder>());

      await tester.tap(dropdown);
      await tester.pumpAndSettle();
      await tester.tap(find.text('CNY').last);
      await tester.pumpAndSettle();
      expect(provider.requestedCurrency, 'CNY');
      expect(find.text('CNY'), findsOneWidget);

      // Failed native updates must restore the provider's actual selection.
      provider.fail = true;
      await tester.tap(dropdown);
      await tester.pumpAndSettle();
      await tester.tap(find.text('EUR').last);
      await tester.pumpAndSettle();
      expect(provider.requestedCurrency, 'CNY');
      expect(find.text('CNY'), findsOneWidget);
      expect(find.textContaining('切换货币失败'), findsOneWidget);

      provider.currencyBusy = true;
      provider.applyCurrencyStatus({'display': 'USD', 'usd_to': 1.0});
      await tester.pumpAndSettle();
      expect(tester.widget<DropdownButton<String>>(dropdown).onChanged, isNull);
      expect(find.text('USD'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  }
}
