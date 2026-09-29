import 'dart:math' as math;
import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../../core/models.dart';
import '../../core/theme.dart';

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

class _YAxisConfig {
  final double maxY;
  final double interval;

  const _YAxisConfig({required this.maxY, required this.interval});
}

class _TrendChartState extends State<TrendChart> {
  int _touchedIndex = -1;

  /// Computes a clean, aesthetically pleasing maxY and step interval (Nice Numbers algorithm).
  /// This prevents awkward ticks like "2736.0M" colliding with "2500.0M".
  _YAxisConfig _computeYAxis(double rawMax) {
    if (rawMax <= 0) {
      return const _YAxisConfig(maxY: 100, interval: 25);
    }
    // Reserve ~12% headroom so the tallest bar does not touch the ceiling
    final targetMax = rawMax * 1.12;
    // Aim for 4 to 5 nice horizontal divisions
    final roughInterval = targetMax / 4.0;
    final exponent = (math.log(roughInterval) / math.ln10).floor();
    final magnitude = math.pow(10, exponent).toDouble();
    final fraction = roughInterval / magnitude;

    double niceInterval;
    if (fraction <= 1.5) {
      niceInterval = 1.0 * magnitude;
    } else if (fraction <= 3.0) {
      niceInterval = 2.0 * magnitude;
    } else if (fraction <= 7.0) {
      niceInterval = 5.0 * magnitude;
    } else {
      niceInterval = 10.0 * magnitude;
    }

    final numIntervals = (targetMax / niceInterval).ceil().clamp(3, 6);
    final niceMaxY = numIntervals * niceInterval;

    return _YAxisConfig(maxY: niceMaxY, interval: niceInterval);
  }

  /// Clean, compact axis number formatter without trailing decimal zeroes
  String _formatAxisNumber(double val) {
    if (val >= 1e12) {
      final t = val / 1e12;
      return t == t.roundToDouble() ? '${t.toInt()}T' : '${t.toStringAsFixed(1)}T';
    } else if (val >= 1e9) {
      final b = val / 1e9;
      return b == b.roundToDouble() ? '${b.toInt()}B' : '${b.toStringAsFixed(1)}B';
    } else if (val >= 1e6) {
      final m = val / 1e6;
      return m == m.roundToDouble() ? '${m.toInt()}M' : '${m.toStringAsFixed(1)}M';
    } else if (val >= 1e3) {
      final k = val / 1e3;
      return k == k.roundToDouble() ? '${k.toInt()}K' : '${k.toStringAsFixed(1)}K';
    }
    return val.toInt().toString();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final themeProvider = Provider.of<ThemeProvider>(context);
    final isDark = theme.brightness == Brightness.dark;
    final primaryColor = theme.colorScheme.primary;

    if (widget.daily.isEmpty) {
      return Card(
        child: Container(
          height: 260,
          alignment: Alignment.center,
          child: Text(
            "暂无趋势数据",
            style: TextStyle(color: theme.colorScheme.outline),
          ),
        ),
      );
    }

    final maxTokens = widget.daily.fold<int>(0, (m, b) => b.tokens > m ? b.tokens : m);
    final yConfig = _computeYAxis(maxTokens.toDouble());

    return Card(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 20.0, vertical: 18.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Header Row with Title and Dynamic Badge Pill
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Row(
                  children: [
                    Icon(
                      Icons.bar_chart_rounded,
                      size: 20,
                      color: primaryColor,
                    ),
                    const SizedBox(width: 8),
                    Text(
                      widget.title,
                      style: theme.textTheme.titleMedium?.copyWith(
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                  ],
                ),
                AnimatedContainer(
                  duration: const Duration(milliseconds: 200),
                  padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                  decoration: BoxDecoration(
                    color: _touchedIndex >= 0 && _touchedIndex < widget.daily.length
                        ? theme.colorScheme.primaryContainer.withOpacity(0.5)
                        : Colors.transparent,
                    borderRadius: BorderRadius.circular(12),
                  ),
                  child: Text(
                    _touchedIndex >= 0 && _touchedIndex < widget.daily.length
                        ? '${widget.daily[_touchedIndex].date} · ${themeProvider.formatTokens(widget.daily[_touchedIndex].tokens)} Tokens · \$${widget.daily[_touchedIndex].costUsd.toStringAsFixed(3)}'
                        : '悬停柱状图查看每日明细',
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: _touchedIndex >= 0
                          ? theme.colorScheme.onPrimaryContainer
                          : theme.colorScheme.outline,
                      fontWeight: _touchedIndex >= 0 ? FontWeight.bold : FontWeight.normal,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 22),

            // Chart Body with Responsive Bar Width & Spacing
            LayoutBuilder(
              builder: (context, constraints) {
                final count = widget.daily.length;
                final availableWidth = (constraints.maxWidth - 52).clamp(100.0, double.infinity);
                final slotWidth = availableWidth / (count > 0 ? count : 1);

                // Responsive bar width: full & solid for <= 7 days, comfortably scaled for 30+ days
                double barWidth;
                if (count <= 7) {
                  barWidth = (slotWidth * 0.35).clamp(26.0, 36.0);
                } else if (count <= 14) {
                  barWidth = (slotWidth * 0.40).clamp(16.0, 24.0);
                } else if (count <= 31) {
                  barWidth = (slotWidth * 0.45).clamp(8.0, 14.0);
                } else {
                  barWidth = (slotWidth * 0.50).clamp(3.0, 8.0);
                }

                final rodBorderRadius = BorderRadius.vertical(
                  top: Radius.circular((barWidth * 0.35).clamp(3.0, 8.0)),
                );

                return SizedBox(
                  height: 210,
                  child: BarChart(
                    BarChartData(
                      maxY: yConfig.maxY,
                      minY: 0,
                      alignment: BarChartAlignment.spaceAround,
                      barTouchData: BarTouchData(
                        touchTooltipData: BarTouchTooltipData(
                          tooltipRoundedRadius: 8,
                          tooltipPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                          tooltipMargin: 6,
                          fitInsideHorizontally: true,
                          fitInsideVertically: true,
                          getTooltipColor: (_) => isDark
                              ? const Color(0xFF242C38)
                              : const Color(0xFF1E293B),
                          getTooltipItem: (group, groupIndex, rod, rodIndex) {
                            final bucket = widget.daily[groupIndex];
                            return BarTooltipItem(
                              '${bucket.date}\n',
                              const TextStyle(
                                color: Colors.white70,
                                fontSize: 11,
                                fontWeight: FontWeight.w500,
                              ),
                              children: [
                                TextSpan(
                                  text: '${themeProvider.formatTokens(bucket.tokens)} Tokens\n',
                                  style: const TextStyle(
                                    color: Colors.white,
                                    fontSize: 12,
                                    fontWeight: FontWeight.bold,
                                  ),
                                ),
                                TextSpan(
                                  text: '成本: \$${bucket.costUsd.toStringAsFixed(3)}',
                                  style: const TextStyle(
                                    color: Color(0xFF4ADE80), // Soft Pastel Green
                                    fontSize: 11,
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                              ],
                            );
                          },
                        ),
                        touchCallback: (event, response) {
                          final newIndex = (!event.isInterestedForInteractions ||
                                  response == null ||
                                  response.spot == null)
                              ? -1
                              : response.spot!.touchedBarGroupIndex;
                          if (_touchedIndex != newIndex) {
                            setState(() {
                              _touchedIndex = newIndex;
                            });
                          }
                        },
                      ),
                      titlesData: FlTitlesData(
                        show: true,
                        topTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                        rightTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                        leftTitles: AxisTitles(
                          sideTitles: SideTitles(
                            showTitles: true,
                            interval: yConfig.interval,
                            reservedSize: 46,
                            getTitlesWidget: (value, meta) {
                              if (value <= 0) return const SizedBox.shrink();
                              // Filter out irregular ticks so only clean interval labels display
                              final rem = value % yConfig.interval;
                              final isCleanTick = rem < 0.1 || (yConfig.interval - rem) < 0.1;
                              if (!isCleanTick) return const SizedBox.shrink();

                              return Padding(
                                padding: const EdgeInsets.only(right: 6.0),
                                child: Text(
                                  _formatAxisNumber(value),
                                  textAlign: TextAlign.right,
                                  style: TextStyle(
                                    fontSize: 10,
                                    color: theme.colorScheme.outline,
                                  ),
                                ),
                              );
                            },
                          ),
                        ),
                        bottomTitles: AxisTitles(
                          sideTitles: SideTitles(
                            showTitles: true,
                            reservedSize: 30,
                            getTitlesWidget: (value, meta) {
                              final index = value.toInt();
                              if (index < 0 || index >= widget.daily.length) return const SizedBox.shrink();
                              final dateStr = widget.daily[index].date;
                              final step = (widget.daily.length / 7).ceil();
                              if (index % step != 0 && index != widget.daily.length - 1) {
                                return const SizedBox.shrink();
                              }
                              final isTouched = index == _touchedIndex;
                              final display = dateStr.length >= 5 ? dateStr.substring(dateStr.length - 5) : dateStr;
                              return Padding(
                                padding: const EdgeInsets.only(top: 8.0),
                                child: Text(
                                  display,
                                  style: TextStyle(
                                    fontSize: 10,
                                    fontWeight: isTouched ? FontWeight.bold : FontWeight.normal,
                                    color: isTouched ? primaryColor : theme.colorScheme.outline,
                                  ),
                                ),
                              );
                            },
                          ),
                        ),
                      ),
                      gridData: FlGridData(
                        show: true,
                        drawVerticalLine: false,
                        checkToShowHorizontalLine: (value) {
                          if (value <= 0) return false;
                          final rem = value % yConfig.interval;
                          return rem < 0.1 || (yConfig.interval - rem) < 0.1;
                        },
                        getDrawingHorizontalLine: (value) => FlLine(
                          color: theme.colorScheme.outlineVariant.withOpacity(0.35),
                          strokeWidth: 1,
                          dashArray: [4, 4],
                        ),
                      ),
                      borderData: FlBorderData(show: false),
                      barGroups: List.generate(widget.daily.length, (i) {
                        final bucket = widget.daily[i];
                        final isTouched = i == _touchedIndex;
                        // Minimum visual height ensures non-zero days are visible and clean
                        final minVisualHeight = yConfig.maxY * 0.015;
                        final displayY = bucket.tokens > 0
                            ? math.max(bucket.tokens.toDouble(), minVisualHeight)
                            : 0.0;

                        return BarChartGroupData(
                          x: i,
                          barRods: [
                            BarChartRodData(
                              toY: displayY,
                              gradient: LinearGradient(
                                begin: Alignment.bottomCenter,
                                end: Alignment.topCenter,
                                colors: isTouched
                                    ? [
                                        primaryColor,
                                        Color.lerp(primaryColor, Colors.white, isDark ? 0.35 : 0.2)!,
                                      ]
                                    : [
                                        primaryColor.withOpacity(0.72),
                                        primaryColor,
                                      ],
                              ),
                              width: barWidth,
                              borderRadius: rodBorderRadius,
                              backDrawRodData: BackgroundBarChartRodData(
                                show: true,
                                toY: yConfig.maxY,
                                color: isTouched
                                    ? primaryColor.withOpacity(0.12)
                                    : (isDark
                                        ? Colors.white.withOpacity(0.04)
                                        : primaryColor.withOpacity(0.045)),
                              ),
                            ),
                          ],
                        );
                      }),
                    ),
                    swapAnimationDuration: const Duration(milliseconds: 250),
                  ),
                );
              },
            ),
          ],
        ),
      ),
    );
  }
}
