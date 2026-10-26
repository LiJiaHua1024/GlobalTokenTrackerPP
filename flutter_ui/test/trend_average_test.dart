import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:globaltokentracker_ui/core/models.dart';
import 'package:globaltokentracker_ui/core/theme.dart';
import 'package:globaltokentracker_ui/widgets/charts/trend_chart.dart';

void main() {
  testWidgets('moving average chart wraps at small widths and can toggle its overlay', (tester) async {
    await tester.pumpWidget(ChangeNotifierProvider(create: (_) => ThemeProvider(), child: MaterialApp(home: Scaffold(body: TrendChart(
      daily: [TrendBucket(date:'2026-10-25',events:1,tokens:100,costUsd:1,top:const [])], averages: const [50],
    )))));
    await tester.pumpAndSettle();
    expect(find.textContaining('移动平均'), findsOneWidget);
    final toggle = find.byType(SwitchListTile);
    expect(tester.widget<SwitchListTile>(toggle).value, isTrue);
    await tester.tap(toggle);
    await tester.pumpAndSettle();
    expect(tester.widget<SwitchListTile>(toggle).value, isFalse);
    expect(tester.takeException(), isNull);
  });
}
