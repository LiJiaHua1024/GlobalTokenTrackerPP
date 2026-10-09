import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:globaltokentracker_ui/core/models.dart';
import 'package:globaltokentracker_ui/core/theme.dart';
import 'package:globaltokentracker_ui/widgets/token_rate_card.dart';

void main() {
  group('TokenRateOverview Models Tests', () {
    test('TokenRateOverview parses full json properly', () {
      final json = {
        'anchor_ms': 1700000000000,
        'latest_event_ms': 1699999950000,
        'is_active': true,
        'm1': {
          'window': '1m',
          'window_secs': 60,
          'start_ms': 1699999940000,
          'end_ms': 1700000000000,
          'events': 15,
          'total_tokens': 120000,
          'input_tokens': 100000,
          'output_tokens': 20000,
          'reasoning_tokens': 5000,
          'cache_read_tokens': 40000,
          'cache_write_tokens': 1000,
          'cost_usd': 0.85,
          'tokens_per_min': 120000.0,
          'tokens_per_sec': 2000.0,
          'input_tokens_per_min': 100000.0,
          'output_tokens_per_min': 20000.0,
          'cost_per_hour': 51.0,
          'requests_per_min': 15.0,
        },
        'm5': {
          'window': '5m',
          'window_secs': 300,
          'total_tokens': 500000,
          'tokens_per_min': 100000.0,
          'tokens_per_sec': 1666.6,
          'cost_usd': 3.5,
          'cost_per_hour': 42.0,
          'requests_per_min': 12.0,
        },
        'm15': {
          'window': '15m',
          'total_tokens': 1200000,
          'tokens_per_min': 80000.0,
        },
        'h1': {
          'window': '1h',
          'total_tokens': 4500000,
          'tokens_per_min': 75000.0,
          'tokens_per_sec': 1250.0,
          'cost_usd': 32.0,
          'cost_per_hour': 32.0,
          'requests_per_min': 10.0,
        },
        'h24': {
          'window': '24h',
          'total_tokens': 50000000,
          'tokens_per_min': 34722.0,
        },
        'peak_1m_in_1h': {
          'window': '1m_peak_1h',
          'total_tokens': 300000,
          'tokens_per_min': 300000.0,
          'tokens_per_sec': 5000.0,
        },
        'peak_1m_in_24h': {
          'window': '1m_peak_24h',
          'total_tokens': 450000,
          'tokens_per_min': 450000.0,
          'tokens_per_sec': 75000.0,
        },
        'timeline_1h': [
          {
            'timestamp_ms': 1699996400000,
            'label': '19:00',
            'total_tokens': 50000,
            'tokens_per_sec': 833.3,
            'tokens_per_min': 50000.0,
            'events': 5,
            'cost_usd': 0.35,
          },
          {
            'timestamp_ms': 1699996460000,
            'label': '19:01',
            'total_tokens': 80000,
            'tokens_per_sec': 1333.3,
            'tokens_per_min': 80000.0,
            'events': 8,
            'cost_usd': 0.60,
          },
        ],
        'top_models_1h': [
          {
            'model': 'claude-3-7-sonnet',
            'app': 'claude',
            'events': 25,
            'total_tokens': 3000000,
            'tokens_per_min': 50000.0,
            'tokens_per_sec': 833.3,
            'cost_usd': 21.0,
            'percentage': 66.7,
          },
          {
            'model': 'gpt-4o',
            'app': 'cursor',
            'events': 15,
            'total_tokens': 1500000,
            'tokens_per_min': 25000.0,
            'tokens_per_sec': 416.7,
            'cost_usd': 11.0,
            'percentage': 33.3,
          },
        ],
        'top_apps_1h': [],
      };

      final overview = TokenRateOverview.fromJson(json);
      expect(overview.anchorMs, 1700000000000);
      expect(overview.isActive, isTrue);
      expect(overview.latestEventMs, 1699999950000);
      expect(overview.m1.totalTokens, 120000);
      expect(overview.m1.tokensPerSec, 2000.0);
      expect(overview.h1.totalTokens, 4500000);
      expect(overview.peak1mIn1h.tokensPerMin, 300000.0);
      expect(overview.timeline1h.length, 2);
      expect(overview.timeline1h[0].label, '19:00');
      expect(overview.topModels1h.length, 2);
      expect(overview.topModels1h[0].model, 'claude-3-7-sonnet');
      expect(overview.topModels1h[0].percentage, 66.7);
    });
  });

  group('TokenRateDashboard Widget Tests', () {
    TokenRateOverview createTestRates({bool isActive = true}) {
      final json = {
        'anchor_ms': 1700000000000,
        'latest_event_ms': 1699999950000,
        'is_active': isActive,
        'm1': {
          'window': '1m',
          'window_secs': 60,
          'total_tokens': 120000,
          'input_tokens': 100000,
          'output_tokens': 20000,
          'cache_read_tokens': 30000,
          'cache_write_tokens': 500,
          'cost_usd': 0.85,
          'tokens_per_min': 120000.0,
          'tokens_per_sec': 2000.0,
          'input_tokens_per_min': 100000.0,
          'output_tokens_per_min': 20000.0,
          'cost_per_hour': 51.0,
          'requests_per_min': 15.0,
        },
        'm5': {
          'window': '5m',
          'total_tokens': 500000,
          'tokens_per_min': 100000.0,
          'tokens_per_sec': 1666.6,
          'cost_usd': 3.5,
          'cost_per_hour': 42.0,
          'requests_per_min': 12.0,
        },
        'm15': {
          'window': '15m',
          'total_tokens': 1200000,
          'tokens_per_min': 80000.0,
          'tokens_per_sec': 1333.3,
          'cost_usd': 8.0,
          'cost_per_hour': 32.0,
          'requests_per_min': 10.0,
        },
        'h1': {
          'window': '1h',
          'total_tokens': 4500000,
          'input_tokens': 3500000,
          'output_tokens': 1000000,
          'cache_read_tokens': 1200000,
          'cache_write_tokens': 50000,
          'tokens_per_min': 75000.0,
          'tokens_per_sec': 1250.0,
          'input_tokens_per_min': 58333.0,
          'output_tokens_per_min': 16667.0,
          'cost_usd': 32.0,
          'cost_per_hour': 32.0,
          'requests_per_min': 10.0,
        },
        'h24': {
          'window': '24h',
          'total_tokens': 50000000,
          'tokens_per_min': 34722.0,
          'tokens_per_sec': 578.7,
          'cost_usd': 280.0,
          'cost_per_hour': 11.67,
          'requests_per_min': 5.0,
        },
        'peak_1m_in_1h': {
          'window': '1m_peak_1h',
          'total_tokens': 300000,
          'tokens_per_min': 300000.0,
          'tokens_per_sec': 5000.0,
        },
        'peak_1m_in_24h': {
          'window': '1m_peak_24h',
          'total_tokens': 450000,
          'tokens_per_min': 450000.0,
          'tokens_per_sec': 7500.0,
        },
        'timeline_1h': List.generate(
          60,
          (i) => {
            'timestamp_ms': 1699996400000 + i * 60000,
            'label': '19:${i.toString().padLeft(2, '0')}',
            'total_tokens': i * 1000,
            'tokens_per_sec': (i * 1000) / 60.0,
            'tokens_per_min': (i * 1000).toDouble(),
            'events': i ~/ 10,
            'cost_usd': i * 0.01,
          },
        ),
        'top_models_1h': [
          {
            'model': 'claude-3-7-sonnet',
            'app': 'claude',
            'events': 25,
            'total_tokens': 3000000,
            'tokens_per_min': 50000.0,
            'tokens_per_sec': 833.3,
            'cost_usd': 21.0,
            'percentage': 66.7,
          },
          {
            'model': 'gpt-4o',
            'app': 'cursor',
            'events': 15,
            'total_tokens': 1500000,
            'tokens_per_min': 25000.0,
            'tokens_per_sec': 416.7,
            'cost_usd': 11.0,
            'percentage': 33.3,
          },
        ],
        'top_apps_1h': [],
      };
      return TokenRateOverview.fromJson(json);
    }

    Widget createTestApp(Widget child) {
      return ChangeNotifierProvider(
        create: (_) => ThemeProvider(),
        child: MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(child: child),
          ),
        ),
      );
    }

    testWidgets('renders header, status, window segments, and KPI cards',
        (tester) async {
      final rates = createTestRates(isActive: true);
      await tester.binding.setSurfaceSize(const Size(1200, 1000));

      await tester.pumpWidget(
        createTestApp(TokenRateDashboard(initialRates: rates)),
      );
      await tester.pump(const Duration(milliseconds: 50));

      // Check header title and active badge
      expect(find.text('Token 增长速率 · 并发实时监控'), findsOneWidget);
      expect(find.text('并发活跃中'), findsOneWidget);

      // Check SegmentedButton window options
      expect(find.text('近 1 分钟'), findsOneWidget);
      expect(find.text('近 5 分钟'), findsOneWidget);
      expect(find.text('近 15 分钟'), findsOneWidget);
      expect(find.text('近 1 小时'), findsOneWidget);
      expect(find.text('近 24 小时'), findsOneWidget);

      // Check KPI card titles
      expect(find.text('Token 增长速率'), findsOneWidget);
      expect(find.text('输入 / 输出 速率分布'), findsOneWidget);
      expect(find.text('预估费用增速'), findsOneWidget);
      expect(find.text('并发请求数与峰值'), findsOneWidget);

      // Check top models section
      expect(find.text('高频并发模型排行'), findsOneWidget);
      expect(find.text('claude-3-7-sonnet'), findsOneWidget);
      expect(find.text('gpt-4o'), findsOneWidget);
    });

    testWidgets('switching time window updates the KPI values', (tester) async {
      final rates = createTestRates(isActive: false);
      await tester.binding.setSurfaceSize(const Size(1200, 1000));

      await tester.pumpWidget(
        createTestApp(TokenRateDashboard(initialRates: rates)),
      );
      await tester.pump(const Duration(milliseconds: 50));

      // Default selected window is '1h'
      expect(find.textContaining('32.00 /h'), findsOneWidget);

      // Tap on '近 1 分钟' tab
      await tester.tap(find.text('近 1 分钟'));
      await tester.pump(const Duration(milliseconds: 50));

      // In 1m, costPerHour is 51.0
      expect(find.textContaining('51.00 /h'), findsOneWidget);
    });

    testWidgets('toggling live monitor switches state properly', (tester) async {
      final rates = createTestRates();
      await tester.binding.setSurfaceSize(const Size(1200, 1000));

      await tester.pumpWidget(
        createTestApp(TokenRateDashboard(initialRates: rates)),
      );
      await tester.pump(const Duration(milliseconds: 50));

      expect(find.text('开启实时监控'), findsOneWidget);

      // Tap toggle chip
      await tester.tap(find.text('开启实时监控'));
      await tester.pump();

      expect(find.text('实时轮询中 (3s)'), findsOneWidget);
    });
  });
}
