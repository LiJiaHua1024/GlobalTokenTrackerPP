import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
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

    // Verify center text shows up in donut mode
    expect(find.textContaining('总计'), findsOneWidget);
  });
}
