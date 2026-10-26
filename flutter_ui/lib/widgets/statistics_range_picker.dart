import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

/// Local calendar boundaries; the end is exclusive, including across DST.
class StatisticsWindow {
  final String key;
  final int? startMs;
  final int? endMs;
  const StatisticsWindow(this.key, {this.startMs, this.endMs});

  factory StatisticsWindow.days(DateTime first, DateTime last,
      {int startHour = 0, int endHour = 23}) {
    final a = first.isBefore(last) ? first : last;
    final b = first.isBefore(last) ? last : first;
    final sameDay = DateUtils.isSameDay(a, b);
    final lo = sameDay ? startHour.clamp(0, 23) : 0;
    final hi = sameDay ? endHour.clamp(lo, 23) : 23;
    return StatisticsWindow('custom',
        startMs: DateTime(a.year, a.month, a.day, lo).millisecondsSinceEpoch,
        endMs: DateTime(b.year, b.month, b.day, hi + 1).millisecondsSinceEpoch);
  }

  String get label {
    if (key != 'custom' || startMs == null || endMs == null) {
      return const {
            'today': '今日',
            'week': '近 7 天',
            'month': '近 30 天',
            'all': '全部'
          }[key] ??
          '自定义';
    }
    final fmt = DateFormat('yyyy/MM/dd HH:mm');
    return '${fmt.format(DateTime.fromMillisecondsSinceEpoch(startMs!))} → ${fmt.format(DateTime.fromMillisecondsSinceEpoch(endMs!))}（结束不含）';
  }
}

class StatisticsRangePicker extends StatelessWidget {
  final StatisticsWindow value;
  final ValueChanged<StatisticsWindow> onChanged;
  const StatisticsRangePicker(
      {super.key, required this.value, required this.onChanged});

  @override
  Widget build(BuildContext context) => Wrap(
        spacing: 8,
        runSpacing: 8,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          for (final key in ['today', 'week', 'month', 'all'])
            ChoiceChip(
                label: Text(StatisticsWindow(key).label),
                selected: value.key == key,
                onSelected: (_) => onChanged(StatisticsWindow(key))),
          ActionChip(
              avatar: const Icon(Icons.date_range, size: 18),
              label: Text(value.key == 'custom' ? '修改自定义区间' : '自定义日期 / 时段'),
              onPressed: () async {
                final picked = await showStatisticsWindowPicker(context, value);
                if (picked != null) onChanged(picked);
              }),
          if (value.key == 'custom')
            Text(value.label, style: Theme.of(context).textTheme.bodySmall),
        ],
      );
}

Future<StatisticsWindow?> showStatisticsWindowPicker(
    BuildContext context, StatisticsWindow initial) {
  final today = DateUtils.dateOnly(DateTime.now());
  var first = initial.startMs == null
      ? today
      : DateUtils.dateOnly(
          DateTime.fromMillisecondsSinceEpoch(initial.startMs!));
  var last = initial.endMs == null
      ? today
      : DateUtils.dateOnly(
          DateTime.fromMillisecondsSinceEpoch(initial.endMs! - 1));
  var startHour = initial.startMs == null
      ? 0
      : DateTime.fromMillisecondsSinceEpoch(initial.startMs!).hour;
  var endHour = initial.endMs == null
      ? 23
      : DateTime.fromMillisecondsSinceEpoch(initial.endMs! - 1).hour;
  return showDialog<StatisticsWindow>(
      context: context,
      builder: (dialogContext) => StatefulBuilder(
            builder: (context, update) => AlertDialog(
              title: const Text('自定义统计区间'),
              content: SizedBox(
                  width: 520,
                  child: SingleChildScrollView(
                      child: Column(
                          mainAxisSize: MainAxisSize.min,
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                        Wrap(spacing: 8, runSpacing: 8, children: [
                          for (final label in [
                            '昨天',
                            '本周',
                            '本月',
                            '上月',
                            '近 90 天'
                          ])
                            ActionChip(
                                label: Text(label),
                                onPressed: () => update(() {
                                      switch (label) {
                                        case '昨天':
                                          first = last = DateTime(today.year,
                                              today.month, today.day - 1);
                                        case '本周':
                                          first = DateTime(
                                              today.year,
                                              today.month,
                                              today.day - today.weekday + 1);
                                          last = today;
                                        case '本月':
                                          first =
                                              DateTime(today.year, today.month);
                                          last = today;
                                        case '上月':
                                          first = DateTime(
                                              today.year, today.month - 1);
                                          last = DateTime(
                                              today.year, today.month, 0);
                                        default:
                                          first = DateTime(today.year,
                                              today.month, today.day - 89);
                                          last = today;
                                      }
                                      startHour = 0;
                                      endHour = 23;
                                    })),
                        ]),
                        const SizedBox(height: 16),
                        OutlinedButton.icon(
                            icon: const Icon(Icons.calendar_month),
                            label: Text(
                                '${DateFormat('yyyy/MM/dd').format(first)} — ${DateFormat('yyyy/MM/dd').format(last)}'),
                            onPressed: () async {
                              final picked = await showDateRangePicker(
                                  context: context,
                                  firstDate: DateTime(2000),
                                  lastDate: today,
                                  initialDateRange:
                                      DateTimeRange(start: first, end: last));
                              if (picked != null) {
                                update(() {
                                  first = picked.start;
                                  last = picked.end;
                                  startHour = 0;
                                  endHour = 23;
                                });
                              }
                            }),
                        const SizedBox(height: 16),
                        if (DateUtils.isSameDay(first, last)) ...[
                          const Text('单日时段（包含所选起止小时）'),
                          const SizedBox(height: 8),
                          Wrap(spacing: 16, runSpacing: 8, children: [
                            DropdownButton<int>(
                                value: startHour,
                                items: [
                                  for (var h = 0; h < 24; h++)
                                    DropdownMenuItem(
                                        value: h,
                                        child: Text(
                                            '起始 ${h.toString().padLeft(2, '0')}:00'))
                                ],
                                onChanged: (h) => update(() {
                                      startHour = h!;
                                      if (endHour < startHour) {
                                        endHour = startHour;
                                      }
                                    })),
                            DropdownButton<int>(
                                value: endHour,
                                items: [
                                  for (var h = startHour; h < 24; h++)
                                    DropdownMenuItem(
                                        value: h,
                                        child: Text(
                                            '结束 ${h.toString().padLeft(2, '0')}:59'))
                                ],
                                onChanged: (h) => update(() {
                                      endHour = h!;
                                    })),
                            TextButton(
                                onPressed: () => update(() {
                                      startHour = 0;
                                      endHour = 23;
                                    }),
                                child: const Text('全天')),
                          ]),
                        ] else
                          const Text('跨日区间按完整自然日统计；选择单日后可指定小时。'),
                      ]))),
              actions: [
                TextButton(
                    onPressed: () => Navigator.pop(dialogContext),
                    child: const Text('取消')),
                FilledButton(
                    onPressed: () => Navigator.pop(
                        dialogContext,
                        StatisticsWindow.days(first, last,
                            startHour: startHour, endHour: endHour)),
                    child: const Text('应用区间'))
              ],
            ),
          ));
}
