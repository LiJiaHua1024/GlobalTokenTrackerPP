import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/pages/speed_ranking_page.dart';
import 'package:globaltokentracker_ui/core/models.dart';

void main() {
  test('event speeds use measured or bounded derived time', () {
    final r = EventRow.fromJson({
      'input_tokens': 100,
      'output_tokens': 50,
      'duration_ms': 1000,
      'ttft_ms': 200
    });
    expect(r.outputTps, 50);
    expect(r.speedDescription, contains('预填 500.0'));
    expect(
        EventRow.fromJson(
            {'ts_start': 0, 'ts_end': 2000000, 'output_tokens': 20}).outputTps,
        isNull);
    expect(
        EventRow.fromJson({'duration_ms': 1000, 'unclassified_tokens': 100})
            .outputTps,
        isNull);
  });

  testWidgets('ranking groups and paginates without layout overflow',
      (tester) async {
    final requests = <Map<String, dynamic>>[];
    await tester.pumpWidget(MaterialApp(
        home: SpeedRankingPage(
            apps: const ['codex'],
            models: const ['model-a'],
            loader: (q) async {
              requests.add(q);
              return {
                'total': 30,
                'page': q['page'],
                'rows': [
                  {
                    'name': 'model-a',
                    'events': 10,
                    'output_tps': 50.0,
                    'speed': {'timed_events': 8, 'speed_est_ms': 100}
                  }
                ]
              };
            })));
    await tester.pumpAndSettle();
    expect(find.textContaining('50.0 tok/s'), findsOneWidget);
    await tester.tap(find.text('按工具'));
    await tester.pumpAndSettle();
    expect(requests.last['group_by'], 'app');
    await tester.tap(find.byIcon(Icons.chevron_right));
    await tester.pumpAndSettle();
    expect(requests.last['page'], 1);
    expect(tester.takeException(), isNull);
  });
}
