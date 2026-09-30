import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';
import '../../core/models.dart';
import '../../core/theme.dart';

class TokenActivityCalendar extends StatefulWidget {
  final List<ActivityDay> activity;
  final String title;

  const TokenActivityCalendar({
    super.key,
    required this.activity,
    this.title = 'Token 活动',
  });

  @override
  State<TokenActivityCalendar> createState() => _TokenActivityCalendarState();
}

class _TokenActivityCalendarState extends State<TokenActivityCalendar> {
  // 'daily' | 'weekly' | 'cumulative'
  String _viewMode = 'daily';

  // Map of "YYYY-MM-DD" -> ActivityDay
  late Map<String, ActivityDay> _activityMap;
  late int _maxDailyTokens;
  late int _maxWeeklyTokens;
  late int _totalYearTokens;
  late int _activeDaysCount;

  // 53 weeks x 7 days grid
  static const int totalWeeks = 53;
  static const int daysPerWeek = 7;

  late DateTime _today;
  late DateTime _startDate;
  // Matrix of [weekIndex][dayIndex] -> DateTime
  late List<List<DateTime>> _gridDates;
  // Month label positions: weekIndex -> "M月"
  late Map<int, String> _monthLabels;

  // Cumulative token lookup: "YYYY-MM-DD" -> cumulative total
  final Map<String, int> _cumulativeTokens = {};
  // Weekly token lookup: weekIndex -> total tokens for that week
  final List<int> _weekTokens = List.filled(totalWeeks, 0);

  // Hovered state for weekly chart
  int? _hoveredWeekIndex;
  // Hovered state for cumulative chart
  int? _hoveredCumIndex;

  @override
  void initState() {
    super.initState();
    _computeGrid();
  }

  @override
  void didUpdateWidget(TokenActivityCalendar oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.activity != widget.activity) {
      _computeGrid();
    }
  }

  void _computeGrid() {
    final now = DateTime.now();
    _today = DateTime(now.year, now.month, now.day);

    _activityMap = {
      for (final a in widget.activity) a.date: a,
    };

    // Calculate start date: 52 full weeks back from the start of this week (Sunday)
    final currentDayOfWeek = _today.weekday % 7; // 0 = Sunday, 1 = Mon, ..., 6 = Sat
    _startDate = _today.subtract(Duration(days: (totalWeeks - 1) * 7 + currentDayOfWeek));

    _gridDates = List.generate(totalWeeks, (w) {
      return List.generate(daysPerWeek, (d) {
        return _startDate.add(Duration(days: w * 7 + d));
      });
    });

    // Detect month transitions with collision prevention (minimum 3 weeks spacing)
    _monthLabels = {};
    int lastMonth = -1;
    int lastWeekCol = -10;
    for (int w = 0; w < totalWeeks; w++) {
      final firstDayInWeek = _gridDates[w][0];
      final month = firstDayInWeek.month;
      if (month != lastMonth) {
        if (lastWeekCol < 0 || (w - lastWeekCol) >= 3) {
          _monthLabels[w] = '$month月';
          lastMonth = month;
          lastWeekCol = w;
        }
      }
    }

    // Compute stats
    _maxDailyTokens = 0;
    _totalYearTokens = 0;
    _activeDaysCount = 0;

    for (final a in widget.activity) {
      if (a.tokens > 0) {
        _activeDaysCount++;
        _totalYearTokens += a.tokens;
        if (a.tokens > _maxDailyTokens) {
          _maxDailyTokens = a.tokens;
        }
      }
    }

    // Compute weekly and cumulative sums
    int runningSum = 0;
    _maxWeeklyTokens = 0;

    for (int w = 0; w < totalWeeks; w++) {
      int weekSum = 0;
      for (int d = 0; d < daysPerWeek; d++) {
        final dt = _gridDates[w][d];
        if (dt.isAfter(_today)) continue;
        final dStr = DateFormat('yyyy-MM-dd').format(dt);
        final act = _activityMap[dStr];
        final t = act?.tokens ?? 0;
        weekSum += t;
        runningSum += t;
        _cumulativeTokens[dStr] = runningSum;
      }
      _weekTokens[w] = weekSum;
      if (weekSum > _maxWeeklyTokens) {
        _maxWeeklyTokens = weekSum;
      }
    }
  }

  Color _getDailyCellColor({
    required DateTime dt,
    required bool isDark,
  }) {
    if (dt.isAfter(_today)) {
      return Colors.transparent;
    }

    final dStr = DateFormat('yyyy-MM-dd').format(dt);
    final act = _activityMap[dStr];
    final dailyTokens = act?.tokens ?? 0;

    final emptyColor = isDark
        ? const Color(0xFF22262B)
        : const Color(0xFFEBEDF0);

    if (dailyTokens <= 0) return emptyColor;

    double intensity = 0.0;
    if (_maxDailyTokens > 0) {
      intensity = (dailyTokens / _maxDailyTokens).clamp(0.0, 1.0);
    }

    // Material 3 progressive blue tones matching the screenshot
    if (intensity <= 0.0) {
      return emptyColor;
    } else if (intensity <= 0.25) {
      return isDark ? const Color(0xFF1E3A5F) : const Color(0xFFBBDEFB);
    } else if (intensity <= 0.50) {
      return isDark ? const Color(0xFF255B9A) : const Color(0xFF64B5F6);
    } else if (intensity <= 0.75) {
      return isDark ? const Color(0xFF2C7BD6) : const Color(0xFF2196F3);
    } else {
      // Highest intensity: bright glowing blue matching screenshot
      return isDark ? const Color(0xFF68B7FF) : const Color(0xFF1976D2);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    final primaryColor = theme.colorScheme.primary;
    final themeProvider = Provider.of<ThemeProvider>(context);

    return Card(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 20.0, vertical: 18.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Header Row: Title & Segmented Button (每日, 每周, 累计)
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Row(
                  children: [
                    Icon(
                      Icons.calendar_today_rounded,
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
                // Sleek Pill Segmented Button
                Container(
                  decoration: BoxDecoration(
                    color: isDark ? const Color(0xFF23272C) : const Color(0xFFF1F3F5),
                    borderRadius: BorderRadius.circular(20),
                    border: Border.all(
                      color: theme.colorScheme.outlineVariant.withOpacity(0.3),
                    ),
                  ),
                  padding: const EdgeInsets.all(2),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      _buildPillButton(
                        label: '每日',
                        value: 'daily',
                        isDark: isDark,
                        theme: theme,
                      ),
                      _buildPillButton(
                        label: '每周',
                        value: 'weekly',
                        isDark: isDark,
                        theme: theme,
                      ),
                      _buildPillButton(
                        label: '累计',
                        value: 'cumulative',
                        isDark: isDark,
                        theme: theme,
                      ),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: 18),

            // Content Area switched smoothly between Daily Calendar, Weekly Bar, Cumulative Area
            AnimatedSwitcher(
              duration: const Duration(milliseconds: 250),
              child: _buildCurrentView(context, isDark, primaryColor, themeProvider, theme),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildCurrentView(
    BuildContext context,
    bool isDark,
    Color primaryColor,
    ThemeProvider themeProvider,
    ThemeData theme,
  ) {
    switch (_viewMode) {
      case 'weekly':
        return _buildWeeklyView(context, isDark, primaryColor, themeProvider, theme);
      case 'cumulative':
        return _buildCumulativeView(context, isDark, primaryColor, themeProvider, theme);
      case 'daily':
      default:
        return _buildDailyHeatmapView(context, isDark, primaryColor, themeProvider, theme);
    }
  }

  /// 1. DAILY VIEW: Classic 7x53 GitHub-style Day-level Matrix
  Widget _buildDailyHeatmapView(
    BuildContext context,
    bool isDark,
    Color primaryColor,
    ThemeProvider themeProvider,
    ThemeData theme,
  ) {
    return Column(
      key: const ValueKey('daily_view'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        LayoutBuilder(
          builder: (context, constraints) {
            final availableWidth = constraints.maxWidth;
            const minCellSize = 9.0;
            const maxCellSize = 14.0;
            const spacing = 2.8;

            final optimalCellSize = ((availableWidth - (totalWeeks - 1) * spacing) / totalWeeks)
                .clamp(minCellSize, maxCellSize);

            return SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  // Matrix: 7 rows x 53 columns
                  Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: List.generate(totalWeeks, (w) {
                      return Padding(
                        padding: EdgeInsets.only(right: w < totalWeeks - 1 ? spacing : 0),
                        child: Column(
                          children: List.generate(daysPerWeek, (d) {
                            final dt = _gridDates[w][d];
                            final isFuture = dt.isAfter(_today);
                            if (isFuture) {
                              return SizedBox(
                                width: optimalCellSize,
                                height: optimalCellSize + (d < daysPerWeek - 1 ? spacing : 0),
                              );
                            }

                            final dStr = DateFormat('yyyy-MM-dd').format(dt);
                            final act = _activityMap[dStr];
                            final dailyTokens = act?.tokens ?? 0;
                            final events = act?.events ?? 0;
                            final cost = act?.costUsd ?? 0.0;

                            final cellColor = _getDailyCellColor(
                              dt: dt,
                              isDark: isDark,
                            );

                            final tooltipText =
                                '$dStr\n${themeProvider.formatTokens(dailyTokens)} Tokens'
                                '${events > 0 ? ' · $events 次请求' : ''}'
                                '${cost > 0 ? ' · \$${cost.toStringAsFixed(3)}' : ''}';

                            return Padding(
                              padding: EdgeInsets.only(bottom: d < daysPerWeek - 1 ? spacing : 0),
                              child: Tooltip(
                                message: tooltipText,
                                waitDuration: const Duration(milliseconds: 150),
                                textStyle: const TextStyle(
                                  color: Colors.white,
                                  fontSize: 12,
                                ),
                                decoration: BoxDecoration(
                                  color: isDark ? const Color(0xFF262D35) : const Color(0xFF1E293B),
                                  borderRadius: BorderRadius.circular(8),
                                  border: Border.all(
                                    color: isDark ? Colors.white12 : Colors.black12,
                                  ),
                                ),
                                child: AnimatedContainer(
                                  duration: const Duration(milliseconds: 200),
                                  width: optimalCellSize,
                                  height: optimalCellSize,
                                  decoration: BoxDecoration(
                                    color: cellColor,
                                    borderRadius: BorderRadius.circular(2.5),
                                  ),
                                ),
                              ),
                            );
                          }),
                        ),
                      );
                    }),
                  ),
                  const SizedBox(height: 10),

                  // Month Labels beneath week columns
                  SizedBox(
                    width: totalWeeks * optimalCellSize + (totalWeeks - 1) * spacing,
                    height: 20,
                    child: Stack(
                      children: _monthLabels.entries.map((entry) {
                        final w = entry.key;
                        final label = entry.value;
                        final leftPos = w * (optimalCellSize + spacing);
                        return Positioned(
                          left: leftPos,
                          child: Text(
                            label,
                            style: TextStyle(
                              fontSize: 11,
                              color: theme.colorScheme.outline,
                              fontWeight: FontWeight.w500,
                            ),
                          ),
                        );
                      }).toList(),
                    ),
                  ),
                ],
              ),
            );
          },
        ),

        const SizedBox(height: 12),

        // Footer Legend Row
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(
              '近一年活跃: $_activeDaysCount 天 · 累计 ${themeProvider.formatTokens(_totalYearTokens)} Tokens',
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.outline,
              ),
            ),
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  '少',
                  style: TextStyle(
                    fontSize: 11,
                    color: theme.colorScheme.outline,
                  ),
                ),
                const SizedBox(width: 6),
                _buildLegendSquare(
                  color: isDark ? const Color(0xFF22262B) : const Color(0xFFEBEDF0),
                ),
                const SizedBox(width: 3),
                _buildLegendSquare(
                  color: isDark ? const Color(0xFF1E3A5F) : const Color(0xFFBBDEFB),
                ),
                const SizedBox(width: 3),
                _buildLegendSquare(
                  color: isDark ? const Color(0xFF255B9A) : const Color(0xFF64B5F6),
                ),
                const SizedBox(width: 3),
                _buildLegendSquare(
                  color: isDark ? const Color(0xFF2C7BD6) : const Color(0xFF2196F3),
                ),
                const SizedBox(width: 3),
                _buildLegendSquare(
                  color: isDark ? const Color(0xFF68B7FF) : const Color(0xFF1976D2),
                ),
                const SizedBox(width: 6),
                Text(
                  '多',
                  style: TextStyle(
                    fontSize: 11,
                    color: theme.colorScheme.outline,
                  ),
                ),
              ],
            ),
          ],
        ),
      ],
    );
  }

  /// 2. WEEKLY VIEW: 52-week Volume Bar Chart (Semantic Weekly Granularity!)
  Widget _buildWeeklyView(
    BuildContext context,
    bool isDark,
    Color primaryColor,
    ThemeProvider themeProvider,
    ThemeData theme,
  ) {
    return Column(
      key: const ValueKey('weekly_view'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        LayoutBuilder(
          builder: (context, constraints) {
            final availableWidth = constraints.maxWidth;
            const barSpacing = 4.0;
            final barWidth = ((availableWidth - (totalWeeks - 1) * barSpacing) / totalWeeks)
                .clamp(6.0, 16.0);

            return SizedBox(
              height: 120,
              child: BarChart(
                BarChartData(
                  maxY: (_maxWeeklyTokens * 1.15).toDouble().clamp(100.0, double.infinity),
                  minY: 0,
                  alignment: BarChartAlignment.spaceBetween,
                  barTouchData: BarTouchData(
                    touchTooltipData: BarTouchTooltipData(
                      tooltipRoundedRadius: 8,
                      fitInsideHorizontally: true,
                      fitInsideVertically: true,
                      getTooltipColor: (_) => isDark ? const Color(0xFF242C38) : const Color(0xFF1E293B),
                      getTooltipItem: (group, groupIndex, rod, rodIndex) {
                        final w = group.x;
                        final startDt = _gridDates[w][0];
                        final endDt = _gridDates[w][daysPerWeek - 1];
                        final fmt = DateFormat('M月d日');
                        final tokens = _weekTokens[w];
                        return BarTooltipItem(
                          '${fmt.format(startDt)} ~ ${fmt.format(endDt)}\n',
                          const TextStyle(color: Colors.white70, fontSize: 11),
                          children: [
                            TextSpan(
                              text: '${themeProvider.formatTokens(tokens)} Tokens',
                              style: const TextStyle(
                                color: Colors.white,
                                fontWeight: FontWeight.bold,
                                fontSize: 13,
                              ),
                            ),
                          ],
                        );
                      },
                    ),
                    touchCallback: (event, response) {
                      setState(() {
                        if (response?.spot != null) {
                          _hoveredWeekIndex = response!.spot!.touchedBarGroupIndex;
                        } else {
                          _hoveredWeekIndex = null;
                        }
                      });
                    },
                  ),
                  titlesData: FlTitlesData(
                    show: true,
                    topTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                    rightTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                    leftTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                    bottomTitles: AxisTitles(
                      sideTitles: SideTitles(
                        showTitles: true,
                        reservedSize: 22,
                        getTitlesWidget: (value, meta) {
                          final w = value.toInt();
                          if (_monthLabels.containsKey(w)) {
                            return Padding(
                              padding: const EdgeInsets.only(top: 4.0),
                              child: Text(
                                _monthLabels[w]!,
                                style: TextStyle(
                                  fontSize: 10,
                                  color: theme.colorScheme.outline,
                                  fontWeight: FontWeight.w500,
                                ),
                              ),
                            );
                          }
                          return const SizedBox.shrink();
                        },
                      ),
                    ),
                  ),
                  gridData: const FlGridData(show: false),
                  borderData: FlBorderData(show: false),
                  barGroups: List.generate(totalWeeks, (w) {
                    final tokens = _weekTokens[w];
                    final isHovered = _hoveredWeekIndex == w;
                    final intensity = _maxWeeklyTokens > 0 ? (tokens / _maxWeeklyTokens).clamp(0.0, 1.0) : 0.0;

                    Color barColor;
                    if (tokens == 0) {
                      barColor = isDark ? const Color(0xFF22262B) : const Color(0xFFEBEDF0);
                    } else if (intensity < 0.3) {
                      barColor = const Color(0xFF2575DC);
                    } else if (intensity < 0.7) {
                      barColor = const Color(0xFF1E88E5);
                    } else {
                      barColor = const Color(0xFF64B5F6);
                    }

                    if (isHovered) {
                      barColor = const Color(0xFF90CAF9);
                    }

                    return BarChartGroupData(
                      x: w,
                      barRods: [
                        BarChartRodData(
                          toY: tokens.toDouble(),
                          color: barColor,
                          width: barWidth,
                          borderRadius: const BorderRadius.vertical(top: Radius.circular(2.5)),
                        ),
                      ],
                    );
                  }),
                ),
              ),
            );
          },
        ),
        const SizedBox(height: 10),
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(
              '按周聚合统计 · 共 53 周 · 周峰值: ${themeProvider.formatTokens(_maxWeeklyTokens)} Tokens',
              style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.outline),
            ),
            Text(
              '悬停查看各周明细',
              style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.outline),
            ),
          ],
        ),
      ],
    );
  }

  /// 3. CUMULATIVE VIEW: Year-round Token Accumulation Growth Curve
  Widget _buildCumulativeView(
    BuildContext context,
    bool isDark,
    Color primaryColor,
    ThemeProvider themeProvider,
    ThemeData theme,
  ) {
    // Generate cumulative curve points across the 53 weeks
    final List<FlSpot> spots = [];
    int runningCum = 0;
    for (int w = 0; w < totalWeeks; w++) {
      for (int d = daysPerWeek - 1; d >= 0; d--) {
        final dt = _gridDates[w][d];
        final dStr = DateFormat('yyyy-MM-dd').format(dt);
        if (_cumulativeTokens.containsKey(dStr)) {
          runningCum = _cumulativeTokens[dStr]!;
          break;
        }
      }
      spots.add(FlSpot(w.toDouble(), runningCum.toDouble()));
    }

    final maxY = _totalYearTokens <= 0 ? 100.0 : (_totalYearTokens * 1.15).toDouble();

    return Column(
      key: const ValueKey('cumulative_view'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          height: 120,
          child: LineChart(
            LineChartData(
              maxY: maxY,
              minY: 0,
              minX: 0,
              maxX: (totalWeeks - 1).toDouble(),
              gridData: FlGridData(
                show: true,
                drawVerticalLine: false,
                horizontalInterval: (maxY / 3).clamp(1.0, double.infinity),
                getDrawingHorizontalLine: (value) => FlLine(
                  color: isDark ? const Color(0xFF282C34) : const Color(0xFFEEEEEE),
                  strokeWidth: 1,
                  dashArray: [4, 4],
                ),
              ),
              titlesData: FlTitlesData(
                show: true,
                topTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                rightTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                leftTitles: AxisTitles(
                  sideTitles: SideTitles(
                    showTitles: true,
                    reservedSize: 46,
                    interval: (maxY / 3).clamp(1.0, double.infinity),
                    getTitlesWidget: (value, meta) {
                      return Text(
                        themeProvider.formatTokens(value),
                        style: TextStyle(
                          fontSize: 9,
                          color: theme.colorScheme.outline,
                        ),
                      );
                    },
                  ),
                ),
                bottomTitles: AxisTitles(
                  sideTitles: SideTitles(
                    showTitles: true,
                    reservedSize: 22,
                    getTitlesWidget: (value, meta) {
                      final w = value.toInt();
                      if (_monthLabels.containsKey(w)) {
                        return Padding(
                          padding: const EdgeInsets.only(top: 4.0),
                          child: Text(
                            _monthLabels[w]!,
                            style: TextStyle(
                              fontSize: 10,
                              color: theme.colorScheme.outline,
                              fontWeight: FontWeight.w500,
                            ),
                          ),
                        );
                      }
                      return const SizedBox.shrink();
                    },
                  ),
                ),
              ),
              borderData: FlBorderData(show: false),
              lineTouchData: LineTouchData(
                enabled: true,
                getTouchLineStart: (barData, spotIndex) => -double.infinity,
                getTouchLineEnd: (barData, spotIndex) => double.infinity,
                getTouchedSpotIndicator: (LineChartBarData barData, List<int> spotIndexes) {
                  return spotIndexes.map((spotIndex) {
                    return TouchedSpotIndicatorData(
                      FlLine(
                        color: isDark ? const Color(0x66FFFFFF) : const Color(0x55000000),
                        strokeWidth: 1.0,
                        dashArray: [6, 4],
                      ),
                      FlDotData(
                        show: true,
                        getDotPainter: (spot, percent, barData, index) {
                          return FlDotCirclePainter(
                            radius: 3.0,
                            color: const Color(0xFF2979FF),
                            strokeWidth: 1.5,
                            strokeColor: isDark ? const Color(0xFF1E242B) : Colors.white,
                          );
                        },
                      ),
                    );
                  }).toList();
                },
                touchTooltipData: LineTouchTooltipData(
                  tooltipRoundedRadius: 8,
                  fitInsideHorizontally: true,
                  fitInsideVertically: true,
                  getTooltipColor: (_) => isDark ? const Color(0xFF242C38) : const Color(0xFF1E293B),
                  getTooltipItems: (touchedSpots) {
                    return touchedSpots.map((spot) {
                      final w = spot.x.toInt();
                      final dt = _gridDates[w.clamp(0, totalWeeks - 1)][daysPerWeek - 1];
                      final displayDt = dt.isAfter(_today) ? _today : dt;
                      final dStr = DateFormat('yyyy年M月d日').format(displayDt);
                      return LineTooltipItem(
                        '截至 $dStr\n',
                        const TextStyle(color: Colors.white70, fontSize: 11),
                        children: [
                          TextSpan(
                            text: '累计: ${themeProvider.formatTokens(spot.y)} Tokens',
                            style: const TextStyle(
                              color: Colors.white,
                              fontWeight: FontWeight.bold,
                              fontSize: 13,
                            ),
                          ),
                        ],
                      );
                    }).toList();
                  },
                ),
              ),
              lineBarsData: [
                LineChartBarData(
                  spots: spots,
                  isCurved: true,
                  curveSmoothness: 0.25,
                  color: const Color(0xFF2979FF),
                  barWidth: 2.5,
                  isStrokeCapRound: true,
                  dotData: const FlDotData(show: false),
                  belowBarData: BarAreaData(
                    show: true,
                    gradient: LinearGradient(
                      begin: Alignment.topCenter,
                      end: Alignment.bottomCenter,
                      colors: [
                        const Color(0xFF2979FF).withOpacity(0.35),
                        const Color(0xFF2979FF).withOpacity(0.0),
                      ],
                    ),
                  ),
                ),
              ],
            ),
            duration: Duration.zero,
          ),
        ),
        const SizedBox(height: 10),
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Text(
              '年度累计用量爬升曲线 · 总计 ${themeProvider.formatTokens(_totalYearTokens)} Tokens',
              style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.outline),
            ),
            Text(
              '悬停查看历史节点累计量',
              style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.outline),
            ),
          ],
        ),
      ],
    );
  }

  Widget _buildPillButton({
    required String label,
    required String value,
    required bool isDark,
    required ThemeData theme,
  }) {
    final isSelected = _viewMode == value;
    return GestureDetector(
      onTap: () {
        if (!isSelected) {
          setState(() => _viewMode = value);
        }
      },
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 180),
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 5),
        decoration: BoxDecoration(
          color: isSelected
              ? (isDark ? const Color(0xFF383D44) : Colors.white)
              : Colors.transparent,
          borderRadius: BorderRadius.circular(16),
          boxShadow: isSelected
              ? [
                  BoxShadow(
                    color: Colors.black.withOpacity(isDark ? 0.25 : 0.08),
                    blurRadius: 4,
                    offset: const Offset(0, 1),
                  ),
                ]
              : null,
        ),
        child: Text(
          label,
          style: TextStyle(
            fontSize: 12,
            fontWeight: isSelected ? FontWeight.bold : FontWeight.w500,
            color: isSelected
                ? (isDark ? Colors.white : theme.colorScheme.primary)
                : theme.colorScheme.outline,
          ),
        ),
      ),
    );
  }

  Widget _buildLegendSquare({required Color color}) {
    return Container(
      width: 10,
      height: 10,
      decoration: BoxDecoration(
        color: color,
        borderRadius: BorderRadius.circular(2),
      ),
    );
  }
}
