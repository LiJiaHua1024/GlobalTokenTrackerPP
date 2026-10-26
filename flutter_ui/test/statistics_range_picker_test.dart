import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/widgets/statistics_range_picker.dart';

void main() {
  test('single-day hours include the end hour and use an exclusive boundary',
      () {
    final d = DateTime(2026, 10, 25);
    final w = StatisticsWindow.days(d, d, startHour: 9, endHour: 11);
    expect(DateTime.fromMillisecondsSinceEpoch(w.startMs!),
        DateTime(2026, 10, 25, 9));
    expect(DateTime.fromMillisecondsSinceEpoch(w.endMs!),
        DateTime(2026, 10, 25, 12));
    final reversed =
        StatisticsWindow.days(DateTime(2026, 10, 25), DateTime(2026, 10, 1));
    expect(DateTime.fromMillisecondsSinceEpoch(reversed.startMs!),
        DateTime(2026, 10, 1));
    expect(DateTime.fromMillisecondsSinceEpoch(reversed.endMs!),
        DateTime(2026, 10, 26));
  });

  testWidgets('custom dialog applies quick dates and fits a small window',
      (tester) async {
    StatisticsWindow? picked;
    await tester.pumpWidget(MaterialApp(
        home: Scaffold(
            body: StatisticsRangePicker(
                value: const StatisticsWindow('week'),
                onChanged: (w) => picked = w))));
    await tester.tap(find.text('自定义日期 / 时段'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('昨天'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('应用区间'));
    await tester.pumpAndSettle();
    final now = DateTime.now();
    expect(DateTime.fromMillisecondsSinceEpoch(picked!.startMs!),
        DateTime(now.year, now.month, now.day - 1));
    expect(DateTime.fromMillisecondsSinceEpoch(picked!.endMs!),
        DateTime(now.year, now.month, now.day));
    expect(tester.takeException(), isNull);
  });
}
