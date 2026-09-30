import 'dart:ui';
import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:globaltokentracker_ui/core/models.dart';
import 'package:globaltokentracker_ui/core/theme.dart';
import 'package:globaltokentracker_ui/widgets/charts/multi_model_trend_chart.dart';
import 'package:globaltokentracker_ui/widgets/charts/token_activity_calendar.dart';

void main() {
  group('TokenActivityCalendar Widget Tests', () {
    testWidgets('renders calendar header, pill toggles, and legend', (WidgetTester tester) async {
      await tester.binding.setSurfaceSize(const Size(1300, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));

      final mockActivity = [
        ActivityDay(date: '2026-09-24', events: 15, tokens: 450000, costUsd: 0.8),
        ActivityDay(date: '2026-09-25', events: 20, tokens: 1200000, costUsd: 2.1),
      ];

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider(create: (_) => ThemeProvider()),
          ],
          child: MaterialApp(
            home: Scaffold(
              body: TokenActivityCalendar(
                activity: mockActivity,
              ),
            ),
          ),
        ),
      );

      // Verify title
      expect(find.text('Token 活动'), findsOneWidget);

      // Verify pill buttons
      expect(find.text('每日'), findsOneWidget);
      expect(find.text('每周'), findsOneWidget);
      expect(find.text('累计'), findsOneWidget);

      // Verify legend
      expect(find.text('少'), findsOneWidget);
      expect(find.text('多'), findsOneWidget);
      expect(find.textContaining('近一年活跃: 2 天'), findsOneWidget);

      // Switch to 每周
      await tester.tap(find.text('每周'));
      await tester.pumpAndSettle();

      // Switch to 累计 and verify cumulative view rendering
      await tester.tap(find.text('累计'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('cumulative_view')), findsOneWidget);
      expect(find.textContaining('年度累计用量爬升曲线'), findsOneWidget);
    });
  });

  group('MultiModelTrendChart Widget Tests', () {
    testWidgets('renders multi-model trend chart with legends and range switch', (WidgetTester tester) async {
      await tester.binding.setSurfaceSize(const Size(1300, 800));
      addTearDown(() => tester.binding.setSurfaceSize(null));

      final mockDaily = [
        TrendBucket(
          date: '2026-09-24',
          events: 20,
          tokens: 1500000,
          costUsd: 2.0,
          top: [
            const MapEntry('stealth/space-bunny-alpha', 1000000),
            const MapEntry('deepseek/deepseek-v4.1-flash', 500000),
          ],
        ),
        TrendBucket(
          date: '2026-09-25',
          events: 30,
          tokens: 2800000,
          costUsd: 3.5,
          top: [
            const MapEntry('stealth/space-bunny-alpha', 2000000),
            const MapEntry('GLM-5.3-Flash', 800000),
          ],
        ),
      ];

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider(create: (_) => ThemeProvider()),
          ],
          child: MaterialApp(
            home: Scaffold(
              body: MultiModelTrendChart(
                daily: mockDaily,
              ),
            ),
          ),
        ),
      );

      // Verify title
      expect(find.text('每日 Token 趋势图'), findsOneWidget);

      // Verify range switch
      expect(find.text('时间范围'), findsOneWidget);
      expect(find.text('近 7 日'), findsOneWidget);
      expect(find.text('近 30 日'), findsOneWidget);

      // Verify model legends from screenshot
      expect(find.text('stealth/space-bunny-alpha'), findsOneWidget);
      expect(find.text('deepseek/deepseek-v4.1-flash'), findsOneWidget);
      expect(find.text('GLM-5.3-Flash'), findsOneWidget);

      // Toggle range to 30d
      await tester.tap(find.text('近 30 日'));
      await tester.pumpAndSettle();

      // Click on a model legend to toggle visibility
      await tester.tap(find.text('GLM-5.3-Flash'));
      await tester.pumpAndSettle();

      // Test hover interaction across chart area
      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);

      final lineChartFinder = find.byType(LineChart);
      final center = tester.getCenter(lineChartFinder);
      await gesture.moveTo(center);
      await tester.pumpAndSettle();

      // Floating tooltip should be displayed near cursor
      expect(find.textContaining('Tokens'), findsWidgets);
    });

    testWidgets('renders empty state when daily data is empty', (WidgetTester tester) async {
      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider(create: (_) => ThemeProvider()),
          ],
          child: const MaterialApp(
            home: Scaffold(
              body: MultiModelTrendChart(
                daily: [],
              ),
            ),
          ),
        ),
      );

      expect(find.text('暂无趋势数据'), findsOneWidget);
    });
  });
}
