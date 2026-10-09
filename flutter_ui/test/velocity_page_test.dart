import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:globaltokentracker_ui/core/models.dart';
import 'package:globaltokentracker_ui/core/theme.dart';
import 'package:globaltokentracker_ui/pages/velocity_page.dart';

void main() {
  TokenRateOverview createTestRates({bool isActive = true}) {
    final json = {
      'anchor_ms': 1700000000000,
      'latest_event_ms': 1699999950000,
      'is_active': isActive,
      'm1': {
        'window': '1m',
        'window_secs': 60,
        'start_ms': 1699999940000,
        'end_ms': 1700000000000,
        'events': 20,
        'total_tokens': 150000,
        'input_tokens': 120000,
        'output_tokens': 30000,
        'reasoning_tokens': 8000,
        'cache_read_tokens': 50000,
        'cache_write_tokens': 2000,
        'cost_usd': 1.15,
        'tokens_per_min': 150000.0,
        'tokens_per_sec': 2500.0,
        'input_tokens_per_min': 120000.0,
        'output_tokens_per_min': 30000.0,
        'cost_per_hour': 69.0,
        'requests_per_min': 20.0,
      },
      'm5': {
        'window': '5m',
        'window_secs': 300,
        'total_tokens': 600000,
        'tokens_per_min': 120000.0,
        'tokens_per_sec': 2000.0,
        'cost_usd': 4.5,
        'cost_per_hour': 54.0,
        'requests_per_min': 15.0,
      },
      'm15': {
        'window': '15m',
        'total_tokens': 1500000,
        'tokens_per_min': 100000.0,
        'tokens_per_sec': 1666.6,
        'cost_usd': 10.5,
        'cost_per_hour': 42.0,
        'requests_per_min': 12.0,
      },
      'h1': {
        'window': '1h',
        'window_secs': 3600,
        'total_tokens': 5000000,
        'input_tokens': 4000000,
        'output_tokens': 1000000,
        'cache_read_tokens': 1500000,
        'cache_write_tokens': 60000,
        'tokens_per_min': 83333.3,
        'tokens_per_sec': 1388.9,
        'input_tokens_per_min': 66666.7,
        'output_tokens_per_min': 16666.7,
        'cost_usd': 35.0,
        'cost_per_hour': 35.0,
        'requests_per_min': 10.0,
      },
      'h24': {
        'window': '24h',
        'window_secs': 86400,
        'total_tokens': 60000000,
        'tokens_per_min': 41666.7,
        'tokens_per_sec': 694.4,
        'cost_usd': 320.0,
        'cost_per_hour': 13.33,
        'requests_per_min': 6.0,
      },
      'peak_1m_in_1h': {
        'window': '1m_peak_1h',
        'total_tokens': 400000,
        'tokens_per_min': 400000.0,
        'tokens_per_sec': 6666.7,
        'events': 12,
        'cost_usd': 2.8,
      },
      'peak_1m_in_24h': {
        'window': '1m_peak_24h',
        'total_tokens': 600000,
        'tokens_per_min': 600000.0,
        'tokens_per_sec': 10000.0,
        'events': 18,
        'cost_usd': 4.2,
      },
      'timeline_1h': List.generate(
        60,
        (i) => {
          'timestamp_ms': 1699996400000 + i * 60000,
          'label': '19:${i.toString().padLeft(2, '0')}',
          'total_tokens': i * 2000,
          'tokens_per_sec': (i * 2000) / 60.0,
          'tokens_per_min': (i * 2000).toDouble(),
          'events': i ~/ 5,
          'cost_usd': i * 0.02,
        },
      ),
      'top_models_1h': [
        {
          'model': 'claude-3-7-sonnet',
          'app': 'claude',
          'events': 30,
          'total_tokens': 3500000,
          'tokens_per_min': 58333.3,
          'tokens_per_sec': 972.2,
          'cost_usd': 25.0,
          'percentage': 70.0,
        },
        {
          'model': 'gpt-4o',
          'app': 'cursor',
          'events': 15,
          'total_tokens': 1500000,
          'tokens_per_min': 25000.0,
          'tokens_per_sec': 416.7,
          'cost_usd': 10.0,
          'percentage': 30.0,
        },
      ],
      'top_apps_1h': [
        {
          'model': '',
          'app': 'Claude Code',
          'events': 30,
          'total_tokens': 3500000,
          'tokens_per_min': 58333.3,
          'tokens_per_sec': 972.2,
          'cost_usd': 25.0,
          'percentage': 70.0,
        },
        {
          'model': '',
          'app': 'Cursor IDE',
          'events': 15,
          'total_tokens': 1500000,
          'tokens_per_min': 25000.0,
          'tokens_per_sec': 416.7,
          'cost_usd': 10.0,
          'percentage': 30.0,
        },
      ],
    };
    return TokenRateOverview.fromJson(json);
  }

  Widget createTestApp(Widget child) {
    return ChangeNotifierProvider(
      create: (_) => ThemeProvider(),
      child: MaterialApp(
        home: Scaffold(
          body: child,
        ),
      ),
    );
  }

  group('VelocityPage Widget Tests', () {
    testWidgets('renders full page with header, pills, cards, chart, and top models',
        (tester) async {
      final rates = createTestRates(isActive: true);
      await tester.binding.setSurfaceSize(const Size(1400, 1100));

      await tester.pumpWidget(
        createTestApp(
          VelocityPage(
            initialRates: rates,
            initialApps: const ['Claude Code', 'Cursor IDE'],
            initialModels: const ['claude-3-7-sonnet', 'gpt-4o'],
          ),
        ),
      );
      await tester.pump(const Duration(milliseconds: 50));

      // Header title and active indicator
      expect(find.text('Token 增长速率与并发监控'), findsOneWidget);
      expect(find.textContaining('实时统计多并发运行时的 Token 吞吐率'), findsOneWidget);
      expect(find.textContaining('并发活跃中'), findsOneWidget);

      // Window pills
      expect(find.text('近 1 分钟'), findsOneWidget);
      expect(find.text('近 5 分钟'), findsOneWidget);
      expect(find.text('近 15 分钟'), findsOneWidget);
      expect(find.text('近 1 小时'), findsOneWidget);
      expect(find.text('近 24 小时'), findsOneWidget);

      // Filter chips
      expect(find.text('工具过滤: '), findsOneWidget);
      expect(find.text('Claude Code'), findsWidgets);
      expect(find.text('Cursor IDE'), findsWidgets);

      // KPI card titles
      expect(find.text('Token 增长速率'), findsOneWidget);
      expect(find.text('输入 / 输出 速率分布'), findsOneWidget);
      expect(find.text('预估费用增速'), findsOneWidget);
      expect(find.text('并发请求数与峰值'), findsOneWidget);

      // 60-min timeline card
      expect(find.textContaining('近 1 小时分钟级速率走势'), findsOneWidget);

      // Top active models and apps
      expect(find.textContaining('高频并发模型排行'), findsOneWidget);
      expect(find.textContaining('并发工具速率分布'), findsOneWidget);
      expect(find.text('claude-3-7-sonnet'), findsOneWidget);
      expect(find.text('Cursor IDE'), findsWidgets);

      // Peak burst cards
      expect(find.textContaining('并发爆发极值洞察'), findsOneWidget);
    });

    testWidgets('switching time windows updates displayed KPI metrics',
        (tester) async {
      final rates = createTestRates(isActive: false);
      await tester.binding.setSurfaceSize(const Size(1400, 1100));

      await tester.pumpWidget(
        createTestApp(
          VelocityPage(
            initialRates: rates,
            initialApps: const ['Claude Code'],
            initialModels: const ['claude-3-7-sonnet'],
          ),
        ),
      );
      await tester.pump(const Duration(milliseconds: 50));

      // Default is 1h: cost per hour is 35.00
      expect(find.textContaining('35.00 /h'), findsOneWidget);

      // Switch to 1m
      await tester.tap(find.text('近 1 分钟'));
      await tester.pump(const Duration(milliseconds: 50));

      // In 1m: cost per hour is 69.00
      expect(find.textContaining('69.00 /h'), findsOneWidget);
    });
  });
}
