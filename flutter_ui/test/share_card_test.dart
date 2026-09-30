import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:globaltokentracker_ui/core/models.dart';
import 'package:globaltokentracker_ui/core/theme.dart';
import 'package:globaltokentracker_ui/widgets/share_card.dart';

OverviewData createMockOverviewData({
  List<ActivityDay>? activity,
  List<TrendBucket>? daily,
}) {
  final span = Totals(
    events: 342,
    inputTokens: 12000000,
    outputTokens: 4500000,
    reasoningTokens: 1500000,
    cacheReadTokens: 2000000,
    cacheWriteTokens: 500000,
    credits: 12.5,
    costUsd: 14.80,
    activeMs: 3600000 * 2, // 2 hours
  );

  final today = Totals(
    events: 50,
    inputTokens: 2000000,
    outputTokens: 800000,
    reasoningTokens: 200000,
    cacheReadTokens: 400000,
    cacheWriteTokens: 100000,
    credits: 0.0,
    costUsd: 2.50,
    activeMs: 3600000,
  );

  return OverviewData(
    today: today,
    span: span,
    all: span,
    rangeType: 'week',
    byApp: [
      AppSummary(
        app: 'Claude Desktop',
        events: 180,
        inputTokens: 7000000,
        outputTokens: 2500000,
        reasoningTokens: 800000,
        cacheReadTokens: 1200000,
        cacheWriteTokens: 300000,
        credits: 10.0,
        costUsd: 8.50,
      ),
      AppSummary(
        app: 'Cursor',
        events: 162,
        inputTokens: 5000000,
        outputTokens: 2000000,
        reasoningTokens: 700000,
        cacheReadTokens: 800000,
        cacheWriteTokens: 200000,
        credits: 2.5,
        costUsd: 6.30,
      ),
    ],
    byModel: [
      ShareRow(name: 'claude-3-5-sonnet', events: 200, tokens: 11000000, costUsd: 9.20),
      ShareRow(name: 'gpt-4o', events: 142, tokens: 7500000, costUsd: 5.60),
    ],
    daily: daily ?? [
      TrendBucket(date: '2026-09-27', events: 120, tokens: 6000000, costUsd: 4.8, top: []),
      TrendBucket(date: '2026-09-28', events: 110, tokens: 5500000, costUsd: 4.5, top: []),
      TrendBucket(date: '2026-09-29', events: 112, tokens: 7000000, costUsd: 5.5, top: []),
    ],
    activity: activity ?? const [],
    apps: ['Claude Desktop', 'Cursor'],
    models: ['claude-3-5-sonnet', 'gpt-4o'],
    quotas: [],
    quotaGroups: [],
    tzOffset: '+08:00',
  );
}

void main() {
  testWidgets('ShareStatCard renders all key stats and branding elements', (WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final mockData = createMockOverviewData();

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '近 7 天统计',
                style: ShareCardStyle.m3Dynamic,
                customHandle: 'alex_engineer',
                showCost: true,
                showActivity: true,
                showModels: true,
                showTools: true,
                showTrend: true,
              ),
            ),
          ),
        ),
      ),
    );

    // Verify Brand title & handle
    expect(find.text('GlobalTokenTracker++'), findsOneWidget);
    expect(find.text('近 7 天统计'), findsOneWidget);
    expect(find.text('@alex_engineer'), findsOneWidget);

    // Verify total token hero value (12M in + 4.5M out + 2M cache = 18.5M)
    expect(find.text('18.50M'), findsOneWidget);

    // Verify metrics tiles
    expect(find.text('预估费用'), findsOneWidget);
    expect(find.text('\$14.80'), findsOneWidget);
    expect(find.text('请求频次'), findsOneWidget);
    expect(find.text('缓存命中率'), findsOneWidget);

    // Verify models and tools
    expect(find.text('claude-3-5-sonnet'), findsOneWidget);
    expect(find.text('gpt-4o'), findsOneWidget);
    expect(find.textContaining('Claude Desktop'), findsOneWidget);
    expect(find.textContaining('Cursor'), findsOneWidget);

    // Verify trend section
    expect(find.text('每日用量趋势 (ACTIVITY RHYTHM)'), findsOneWidget);

    // Verify footer disclaimer
    expect(find.textContaining('本地安全统计'), findsOneWidget);
  });

  testWidgets('ShareStatCard honors privacy toggles to hide cost and activity', (WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final mockData = createMockOverviewData();

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '今日用量',
                style: ShareCardStyle.midnightDark,
                showCost: false, // Cost hidden!
                showActivity: false, // Activity hidden!
                showModels: false, // Models hidden!
                showTools: false, // Tools hidden!
                showTrend: false, // Trend hidden!
              ),
            ),
          ),
        ),
      ),
    );

    // Total tokens should still be visible
    expect(find.text('18.50M'), findsOneWidget);

    // Cost and activity should NOT be visible
    expect(find.text('预估费用'), findsNothing);
    expect(find.text('\$14.80'), findsNothing);
    expect(find.text('请求频次'), findsNothing);

    // Models, tools, and trend should NOT be visible
    expect(find.text('claude-3-5-sonnet'), findsNothing);
    expect(find.text('每日用量趋势 (ACTIVITY RHYTHM)'), findsNothing);
  });

  testWidgets('ShareCardDialog can be launched and displays preview with controls', (WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final mockData = createMockOverviewData();

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: Builder(
              builder: (context) {
                return ElevatedButton(
                  onPressed: () => showShareCardDialog(context, mockData, 'week'),
                  child: const Text('打开分享卡片'),
                );
              },
            ),
          ),
        ),
      ),
    );

    // Open the dialog
    await tester.tap(find.text('打开分享卡片'));
    await tester.pumpAndSettle();

    // Verify dialog header & buttons
    expect(find.text('生成用量分享卡片'), findsOneWidget);
    expect(find.text('复制图片到剪贴板'), findsOneWidget);
    expect(find.text('另存为图片...'), findsOneWidget);
    expect(find.text('快速存入相册'), findsOneWidget);
    expect(find.text('复制 Markdown 文本摘要'), findsOneWidget);

    // Verify style choice chips
    expect(find.text('MD3 动态'), findsOneWidget);
    expect(find.text('黑曜夜空'), findsOneWidget);
    expect(find.text('极光幻境'), findsOneWidget);
    expect(find.text('现代极简'), findsOneWidget);

    // Tap different style
    await tester.tap(find.text('黑曜夜空'));
    await tester.pumpAndSettle();

    // Verify close dialog
    await tester.tap(find.byIcon(Icons.close));
    await tester.pumpAndSettle();
    expect(find.text('生成用量分享卡片'), findsNothing);
  });

  testWidgets('ShareStatCard adapts report title, tag, and trend badge dynamically to selected period', (WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final mockData = createMockOverviewData();

    // 1. Test Today (Daily)
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '今日用量',
                rangeKey: 'today',
              ),
            ),
          ),
        ),
      ),
    );

    expect(find.text('AI 用量追踪与成本分析日报'), findsOneWidget);
    expect(find.text('DAILY REPORT'), findsOneWidget);
    expect(find.text('今日时段走势 (HOURLY RHYTHM)'), findsOneWidget);
    expect(find.text('3 个活跃时段'), findsOneWidget);

    // 2. Test Month
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '近 30 天统计',
                rangeKey: 'month',
              ),
            ),
          ),
        ),
      ),
    );

    expect(find.text('AI 用量追踪与成本分析月报'), findsOneWidget);
    expect(find.text('MONTHLY REPORT'), findsOneWidget);
    expect(find.text('每日用量趋势 (ACTIVITY RHYTHM)'), findsOneWidget);
    expect(find.text('3 日走势'), findsOneWidget);

    // 3. Test All-Time
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '全部累计用量',
                rangeKey: 'all',
              ),
            ),
          ),
        ),
      ),
    );

    expect(find.text('AI 用量追踪与成本全景总报'), findsOneWidget);
    expect(find.text('ALL-TIME REPORT'), findsOneWidget);
    expect(find.text('历史用量趋势 (ACTIVITY RHYTHM)'), findsOneWidget);
    expect(find.text('3 日记录'), findsOneWidget);
  });

  testWidgets('ShareStatCard renders token activity heatmap when activity data is available', (WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final mockActivity = [
      ActivityDay(date: '2026-09-24', events: 10, tokens: 500000, costUsd: 1.0),
      ActivityDay(date: '2026-09-25', events: 15, tokens: 800000, costUsd: 1.5),
      ActivityDay(date: '2026-09-26', events: 20, tokens: 1200000, costUsd: 2.0),
    ];

    final mockData = createMockOverviewData(activity: mockActivity);

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '近 30 天统计',
                showHeatmap: true,
              ),
            ),
          ),
        ),
      ),
    );

    // Verify Token Activity Heatmap section elements
    expect(find.text('TOKEN 活动矩阵 (COMMIT HEATMAP)'), findsOneWidget);
    expect(find.text('活跃 3 天'), findsOneWidget);
    expect(find.text('最长连续 3 天'), findsOneWidget);
    expect(find.text('少'), findsOneWidget);
    expect(find.text('多'), findsOneWidget);
    expect(find.textContaining('近一年累计活动活跃'), findsOneWidget);

    // When showHeatmap is false, heatmap should not be rendered
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '近 30 天统计',
                showHeatmap: false,
              ),
            ),
          ),
        ),
      ),
    );

    expect(find.text('TOKEN 活动矩阵 (COMMIT HEATMAP)'), findsNothing);
  });

  testWidgets('ShareStatCard renders multi-model trend chart with line chart and legends', (WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final mockDaily = [
      TrendBucket(
        date: '2026-09-27',
        events: 50,
        tokens: 3000000,
        costUsd: 2.5,
        top: [
          const MapEntry('claude-3-5-sonnet', 2000000),
          const MapEntry('gpt-4o', 1000000),
        ],
      ),
      TrendBucket(
        date: '2026-09-28',
        events: 60,
        tokens: 4000000,
        costUsd: 3.2,
        top: [
          const MapEntry('claude-3-5-sonnet', 2500000),
          const MapEntry('gpt-4o', 1500000),
        ],
      ),
    ];

    final mockData = createMockOverviewData(daily: mockDaily);

    // 1. Multi-model curve mode
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '近 7 天统计',
                trendChartType: ShareTrendChartType.multiModel,
              ),
            ),
          ),
        ),
      ),
    );

    expect(find.byType(LineChart), findsOneWidget);
    expect(find.byIcon(Icons.insights_rounded), findsOneWidget);

    // 2. Classic bars mode
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: ShareStatCard(
                data: mockData,
                periodLabel: '近 7 天统计',
                trendChartType: ShareTrendChartType.classicBars,
              ),
            ),
          ),
        ),
      ),
    );

    expect(find.byType(LineChart), findsNothing);
    expect(find.byIcon(Icons.show_chart), findsOneWidget);
  });

  testWidgets('ShareCardDialog provides toggles for Token Activity Heatmap and Trend Chart Style', (WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final mockData = createMockOverviewData();

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider(create: (_) => ThemeProvider()),
        ],
        child: MaterialApp(
          home: Scaffold(
            body: Builder(
              builder: (context) {
                return ElevatedButton(
                  onPressed: () => showShareCardDialog(context, mockData, 'week'),
                  child: const Text('打开分享卡片'),
                );
              },
            ),
          ),
        ),
      ),
    );

    // Open the dialog
    await tester.tap(find.text('打开分享卡片'));
    await tester.pumpAndSettle();

    // Verify heatmap toggle switch is present
    expect(find.text('显示 Token 活动打卡热力图'), findsOneWidget);

    // Verify trend chart style segmented buttons are present
    expect(find.text('多模型平滑曲线'), findsOneWidget);
    expect(find.text('经典走势柱状'), findsOneWidget);

    // Tap classic bars segmented button
    await tester.ensureVisible(find.text('经典走势柱状'));
    await tester.tap(find.text('经典走势柱状'));
    await tester.pumpAndSettle();

    // Verify close
    await tester.tap(find.byIcon(Icons.close));
    await tester.pumpAndSettle();
  });
}
