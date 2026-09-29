import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../../core/models.dart';

class TrendChart extends StatefulWidget {
  final List<TrendBucket> daily;
  final String title;

  const TrendChart({
    Key? key,
    required this.daily,
    this.title = '用量趋势',
  }) : super(key: key);

  @override
  State<TrendChart> createState() => _TrendChartState();
}

class _TrendChartState extends State<TrendChart> {
  int _touchedIndex = -1;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final primaryColor = theme.colorScheme.primary;

    if (widget.daily.isEmpty) {
      return Card(
        child: Container(
          height: 240,
          alignment: Alignment.center,
          child: Text("暂无趋势数据", style: TextStyle(color: theme.colorScheme.outline)),
        ),
      );
    }

    // Find max tokens for Y-axis scaling
    final maxTokens = widget.daily.fold<int>(0, (m, b) => b.tokens > m ? b.tokens : m);
    final maxY = (maxTokens * 1.15).toDouble().clamp(100.0, double.infinity);

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(20.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text(
                  widget.title,
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.bold,
                  ),
                ),
                Text(
                  _touchedIndex >= 0 && _touchedIndex < widget.daily.length
                      ? '${widget.daily[_touchedIndex].date} · ${NumberFormat('#,###').format(widget.daily[_touchedIndex].tokens)} Tokens · \$${widget.daily[_touchedIndex].costUsd.toStringAsFixed(3)}'
                      : '悬停柱状图查看每日明细',
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: _touchedIndex >= 0 ? theme.colorScheme.primary : theme.colorScheme.outline,
                    fontWeight: _touchedIndex >= 0 ? FontWeight.bold : FontWeight.normal,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 24),
            SizedBox(
              height: 200,
              child: BarChart(
                BarChartData(
                  maxY: maxY,
                  barTouchData: BarTouchData(
                    touchTooltipData: BarTouchTooltipData(
                      getTooltipColor: (_) => theme.colorScheme.surfaceVariant,
                      getTooltipItem: (group, groupIndex, rod, rodIndex) {
                        final bucket = widget.daily[groupIndex];
                        return BarTooltipItem(
                          '${bucket.date}\nTokens: ${NumberFormat('#,###').format(bucket.tokens)}\n成本: \$${bucket.costUsd.toStringAsFixed(3)}',
                          TextStyle(
                            color: theme.colorScheme.onSurfaceVariant,
                            fontWeight: FontWeight.bold,
                            fontSize: 12,
                          ),
                        );
                      },
                    ),
                    touchCallback: (event, response) {
                      setState(() {
                        if (!event.isInterestedForInteractions ||
                            response == null ||
                            response.spot == null) {
                          _touchedIndex = -1;
                          return;
                        }
                        _touchedIndex = response.spot!.touchedBarGroupIndex;
                      });
                    },
                  ),
                  titlesData: FlTitlesData(
                    show: true,
                    topTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                    rightTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                    leftTitles: AxisTitles(
                      sideTitles: SideTitles(
                        showTitles: true,
                        reservedSize: 42,
                        getTitlesWidget: (value, meta) {
                          if (value == 0) return const Text('');
                          String text;
                          if (value >= 1000000) {
                            text = '${(value / 1000000).toStringAsFixed(1)}M';
                          } else if (value >= 1000) {
                            text = '${(value / 1000).toStringAsFixed(0)}K';
                          } else {
                            text = value.toInt().toString();
                          }
                          return Text(text, style: TextStyle(fontSize: 10, color: theme.colorScheme.outline));
                        },
                      ),
                    ),
                    bottomTitles: AxisTitles(
                      sideTitles: SideTitles(
                        showTitles: true,
                        reservedSize: 28,
                        getTitlesWidget: (value, meta) {
                          final index = value.toInt();
                          if (index < 0 || index >= widget.daily.length) return const Text('');
                          final dateStr = widget.daily[index].date;
                          // Show only some labels to avoid overcrowding
                          final step = (widget.daily.length / 7).ceil();
                          if (index % step != 0 && index != widget.daily.length - 1) {
                            return const Text('');
                          }
                          final display = dateStr.length >= 5 ? dateStr.substring(dateStr.length - 5) : dateStr;
                          return Text(display, style: TextStyle(fontSize: 10, color: theme.colorScheme.outline));
                        },
                      ),
                    ),
                  ),
                  gridData: FlGridData(
                    show: true,
                    drawVerticalLine: false,
                    getDrawingHorizontalLine: (value) => FlLine(
                      color: theme.colorScheme.outlineVariant.withOpacity(0.4),
                      strokeWidth: 1,
                    ),
                  ),
                  borderData: FlBorderData(show: false),
                  barGroups: List.generate(widget.daily.length, (i) {
                    final bucket = widget.daily[i];
                    final isTouched = i == _touchedIndex;
                    return BarChartGroupData(
                      x: i,
                      barRods: [
                        BarChartRodData(
                          toY: bucket.tokens.toDouble(),
                          color: isTouched ? theme.colorScheme.tertiary : primaryColor,
                          width: (180.0 / widget.daily.length).clamp(4.0, 18.0),
                          borderRadius: const BorderRadius.vertical(top: Radius.circular(4)),
                        ),
                      ],
                    );
                  }),
                ),
                swapAnimationDuration: const Duration(milliseconds: 300),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
