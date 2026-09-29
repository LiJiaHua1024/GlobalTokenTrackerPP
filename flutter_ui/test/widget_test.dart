import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/theme.dart';
import 'package:globaltokentracker_ui/widgets/charts/pie_donut_chart.dart';

void main() {
  testWidgets('PieDonutChart renders in pie mode and can toggle to donut mode', (WidgetTester tester) async {
    const items = [
      PieDonutChartItem(label: 'Claude', value: 100),
      PieDonutChartItem(label: 'Cursor', value: 200),
    ];

    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: PieDonutChart(
            title: '测试图表',
            items: items,
            initialIsDonut: false,
          ),
        ),
      ),
    );

    // Verify title and chips are rendered
    expect(find.text('测试图表'), findsOneWidget);
    expect(find.textContaining('Claude'), findsOneWidget);
    expect(find.textContaining('Cursor'), findsOneWidget);

    // Tap donut toggle button
    final donutButton = find.byIcon(Icons.donut_large);
    expect(donutButton, findsOneWidget);
    await tester.tap(donutButton);
    await tester.pumpAndSettle();

    // Verify center text and header subtitle show up in donut mode
    expect(find.textContaining('总计'), findsWidgets);
  });

  test('ThemeProvider formats numbers with K, M, B, T and respects toggle', () {
    final theme = ThemeProvider();
    expect(theme.compactNumbers, isTrue);

    expect(theme.formatTokens(450), '450');
    expect(theme.formatTokens(1500), '1.5K');
    expect(theme.formatTokens(2340000), '2.34M');
    expect(theme.formatTokens(5789000000), '5.79B');
    expect(theme.formatTokens(1200000000000), '1.20T');

    theme.setCompactNumbers(false);
    expect(theme.compactNumbers, isFalse);
    expect(theme.formatTokens(2340000), '2,340,000');
  });
}
