import 'dart:math' as math;
import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../../core/models.dart';
import '../../core/theme.dart';

class MultiModelTrendChart extends StatefulWidget {
  final List<TrendBucket> daily;
  final List<TrendBucket>? daily30d;
  final String title;

  const MultiModelTrendChart({
    super.key,
    required this.daily,
    this.daily30d,
    this.title = '每日 Token 趋势图',
  });

  @override
  State<MultiModelTrendChart> createState() => _MultiModelTrendChartState();
}

class _MultiModelTrendChartState extends State<MultiModelTrendChart> {
  // '7d' | '30d'
  String _selectedDays = '7d';

  // Set of models currently enabled/visible
  final Set<String> _hiddenModels = {};
  String? _highlightedModel;

  // Snapped hover state
  int? _hoveredIndex;
  double _mouseY = 60.0;

  // Harmonious, high-contrast M3 palette matching reference screenshot
  static const List<Color> _palette = [
    Color(0xFF2979FF), // Vibrant Blue (stealth)
    Color(0xFF00E676), // Bright Emerald (step-5)
    Color(0xFFB388FF), // Lavender Purple (deepseek)
    Color(0xFFFF5252), // Coral Red (GLM)
    Color(0xFFFF9100), // Tangerine Orange (xiaomi)
    Color(0xFF00E5FF), // Cyan Teal (mimo)
    Color(0xFFFF4081), // Neon Pink
    Color(0xFF7C4DFF), // Deep Violet
    Color(0xFFFFD740), // Bright Amber
    Color(0xFF1DE9B6), // Seafoam Teal
    Color(0xFFA7FFEB), // Mint
    Color(0xFFFF6E40), // Flame
  ];

  Color _getModelColor(String model, int index) {
    return _palette[index % _palette.length];
  }

  /// Selects the buckets list according to `_selectedDays`
  List<TrendBucket> _getActiveBuckets() {
    final source = (_selectedDays == '30d' && widget.daily30d != null && widget.daily30d!.isNotEmpty)
        ? widget.daily30d!
        : widget.daily;

    if (source.isEmpty) return [];

    if (_selectedDays == '7d') {
      final count = source.length;
      final takeCount = math.min(count, 8);
      return source.sublist(count - takeCount);
    }
    return source;
  }

  /// Computes a clean, aesthetically pleasing maxY and step interval (Nice Numbers algorithm).
  double _computeInterval(double rawMax) {
    if (rawMax <= 0) return 100;
    final roughInterval = (rawMax * 1.15) / 4.0;
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
    return niceInterval;
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final isDark = theme.brightness == Brightness.dark;
    final primaryColor = theme.colorScheme.primary;
    final themeProvider = Provider.of<ThemeProvider>(context);

    final activeBuckets = _getActiveBuckets();

    if (activeBuckets.isEmpty) {
      return Card(
        child: Container(
          height: 320,
          alignment: Alignment.center,
          padding: const EdgeInsets.all(24),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(Icons.show_chart_rounded, size: 40, color: theme.colorScheme.outline),
              const SizedBox(height: 12),
              Text(
                '暂无趋势数据',
                style: TextStyle(color: theme.colorScheme.outline),
              ),
            ],
          ),
        ),
      );
    }

    // 1. Collect all distinct models active across activeBuckets
    final Map<String, int> modelTotalTokens = {};
    for (final b in activeBuckets) {
      for (final entry in b.top) {
        final model = entry.key;
        final tokens = entry.value;
        if (tokens > 0) {
          modelTotalTokens[model] = (modelTotalTokens[model] ?? 0) + tokens;
        }
      }
    }

    // Sort models descending by total tokens
    final sortedModels = modelTotalTokens.keys.toList()
      ..sort((a, b) => (modelTotalTokens[b] ?? 0).compareTo(modelTotalTokens[a] ?? 0));

    // Assign consistent color to each model
    final Map<String, Color> modelColors = {};
    for (int i = 0; i < sortedModels.length; i++) {
      modelColors[sortedModels[i]] = _getModelColor(sortedModels[i], i);
    }

    // 2. Prepare series data
    final int numPoints = activeBuckets.length;
    double maxVal = 0;

    // Map: model -> list of token values per point index
    final Map<String, List<int>> modelValues = {};

    for (final model in sortedModels) {
      modelValues[model] = List.filled(numPoints, 0);
      for (int i = 0; i < numPoints; i++) {
        final bucket = activeBuckets[i];
        for (final entry in bucket.top) {
          if (entry.key == model) {
            final t = entry.value;
            modelValues[model]![i] = t;
            if (t > maxVal) maxVal = t.toDouble();
            break;
          }
        }
      }
    }

    final List<LineChartBarData> lineBarsData = [];

    for (final model in sortedModels) {
      if (_hiddenModels.contains(model)) continue;

      final isHighlighted = _highlightedModel == model;
      final isAnyHighlighted = _highlightedModel != null;
      final color = modelColors[model] ?? primaryColor;

      final spots = <FlSpot>[];
      final vals = modelValues[model] ?? [];
      for (int i = 0; i < numPoints; i++) {
        spots.add(FlSpot(i.toDouble(), (vals.length > i ? vals[i] : 0).toDouble()));
      }

      lineBarsData.add(
        LineChartBarData(
          spots: spots,
          showingIndicators: _hoveredIndex != null ? [_hoveredIndex!] : const [],
          isCurved: true,
          curveSmoothness: 0.35,
          preventCurveOverShooting: true,
          color: isAnyHighlighted && !isHighlighted
              ? color.withValues(alpha: 0.18)
              : color,
          barWidth: isHighlighted ? 3.0 : 2.0,
          isStrokeCapRound: true,
          dotData: const FlDotData(show: false),
          belowBarData: BarAreaData(show: false),
        ),
      );
    }

    final double interval = _computeInterval(maxVal);
    final double maxY = maxVal <= 0 ? 100 : (maxVal * 1.18);

    // Layout constants for coordinate geometry
    const double leftReserved = 46.0;
    const double bottomReserved = 28.0;
    const double totalChartHeight = 240.0;

    return Card(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 20.0, vertical: 18.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Top Controls Row: "时间范围" + Pill Switcher
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Row(
                  children: [
                    Icon(
                      Icons.insights_rounded,
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
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      '时间范围',
                      style: TextStyle(
                        fontSize: 12,
                        color: theme.colorScheme.outline,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                    const SizedBox(width: 10),
                    Container(
                      decoration: BoxDecoration(
                        color: isDark ? const Color(0xFF23272C) : const Color(0xFFF1F3F5),
                        borderRadius: BorderRadius.circular(20),
                        border: Border.all(
                          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.3),
                        ),
                      ),
                      padding: const EdgeInsets.all(2),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          _buildRangePill(
                            label: '近 7 日',
                            value: '7d',
                            isDark: isDark,
                            theme: theme,
                          ),
                          _buildRangePill(
                            label: '近 30 日',
                            value: '30d',
                            isDark: isDark,
                            theme: theme,
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ],
            ),
            const SizedBox(height: 16),

            // Model Legends
            if (sortedModels.isNotEmpty) ...[
              Wrap(
                spacing: 16,
                runSpacing: 8,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: sortedModels.map((model) {
                  final color = modelColors[model] ?? primaryColor;
                  final isHidden = _hiddenModels.contains(model);
                  final isHovered = _highlightedModel == model;

                  return MouseRegion(
                    cursor: SystemMouseCursors.click,
                    onEnter: (_) => setState(() => _highlightedModel = model),
                    onExit: (_) => setState(() => _highlightedModel = null),
                    child: GestureDetector(
                      onTap: () {
                        setState(() {
                          if (isHidden) {
                            _hiddenModels.remove(model);
                          } else {
                            if (_hiddenModels.length < sortedModels.length - 1) {
                              _hiddenModels.add(model);
                            }
                          }
                        });
                      },
                      child: AnimatedOpacity(
                        duration: const Duration(milliseconds: 150),
                        opacity: isHidden ? 0.35 : (isHovered ? 1.0 : 0.85),
                        child: Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Container(
                              width: 8,
                              height: 8,
                              decoration: BoxDecoration(
                                shape: BoxShape.circle,
                                color: color,
                                boxShadow: isHovered
                                    ? [
                                        BoxShadow(
                                          color: color.withValues(alpha: 0.6),
                                          blurRadius: 6,
                                          spreadRadius: 1,
                                        ),
                                      ]
                                    : null,
                              ),
                            ),
                            const SizedBox(width: 6),
                            Text(
                              model,
                              style: TextStyle(
                                fontSize: 12,
                                fontWeight: isHovered ? FontWeight.bold : FontWeight.w500,
                                color: isDark ? Colors.white70 : Colors.black87,
                                decoration: isHidden ? TextDecoration.lineThrough : null,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  );
                }).toList(),
              ),
              const SizedBox(height: 20),
            ],

            // Interactive Spline Line Chart Area with Snap-to-Nearest Cursor & Floating Tooltip
            LayoutBuilder(
              builder: (context, constraints) {
                final totalWidth = constraints.maxWidth;
                final plotWidth = (totalWidth - leftReserved).clamp(100.0, double.infinity);

                // Compute exact pixel coordinates for hovered index tooltip
                double? snappedX;
                TrendBucket? hoveredBucket;

                if (_hoveredIndex != null && _hoveredIndex! >= 0 && _hoveredIndex! < numPoints) {
                  final idx = _hoveredIndex!;
                  hoveredBucket = activeBuckets[idx];
                  snappedX = leftReserved + (idx / (numPoints - 1).clamp(1, 9999)) * plotWidth;
                }

                return MouseRegion(
                  onHover: (event) {
                    final localX = event.localPosition.dx;
                    final relX = (localX - leftReserved).clamp(0.0, plotWidth);
                    final fraction = relX / plotWidth;
                    final nearest = (fraction * (numPoints - 1)).round().clamp(0, numPoints - 1);
                    final newY = event.localPosition.dy;
                    if (_hoveredIndex != nearest || (_mouseY - newY).abs() > 3.0) {
                      setState(() {
                        _hoveredIndex = nearest;
                        _mouseY = newY;
                      });
                    }
                  },
                  onExit: (_) {
                    setState(() {
                      _hoveredIndex = null;
                    });
                  },
                  child: SizedBox(
                    height: totalChartHeight,
                    child: Stack(
                      clipBehavior: Clip.none,
                      children: [
                        // Base LineChart with native indicator dots perfectly on curves (Duration.zero ensures instantaneous 0ms response)
                        LineChart(
                          LineChartData(
                            maxY: maxY,
                            minY: 0,
                            minX: 0,
                            maxX: (numPoints - 1).toDouble().clamp(0.0, double.infinity),
                            gridData: FlGridData(
                              show: true,
                              drawVerticalLine: false,
                              horizontalInterval: interval > 0 ? interval : 100,
                              getDrawingHorizontalLine: (value) {
                                return FlLine(
                                  color: isDark
                                      ? const Color(0xFF282C34)
                                      : const Color(0xFFEEEEEE),
                                  strokeWidth: 1,
                                  dashArray: [4, 4],
                                );
                              },
                            ),
                            titlesData: FlTitlesData(
                              show: true,
                              topTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                              rightTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                              leftTitles: AxisTitles(
                                sideTitles: SideTitles(
                                  showTitles: true,
                                  reservedSize: leftReserved,
                                  interval: interval > 0 ? interval : 100,
                                  getTitlesWidget: (value, meta) {
                                    if (value < 0 || value > maxY) {
                                      return const SizedBox.shrink();
                                    }
                                    return Text(
                                      themeProvider.formatTokens(value),
                                      style: TextStyle(
                                        fontSize: 10,
                                        color: theme.colorScheme.outline,
                                      ),
                                    );
                                  },
                                ),
                              ),
                              bottomTitles: AxisTitles(
                                sideTitles: SideTitles(
                                  showTitles: true,
                                  reservedSize: bottomReserved,
                                  interval: _selectedDays == '7d' ? 1.0 : (numPoints > 15 ? 4.0 : 2.0),
                                  getTitlesWidget: (value, meta) {
                                    final idx = value.toInt();
                                    if (idx < 0 || idx >= numPoints) {
                                      return const SizedBox.shrink();
                                    }
                                    final dateStr = activeBuckets[idx].date;
                                    String displayDate = dateStr;
                                    try {
                                      final parsed = DateTime.parse(dateStr);
                                      displayDate = '${parsed.month}月${parsed.day}日';
                                    } catch (_) {
                                      if (dateStr.contains('-')) {
                                        final parts = dateStr.split('-');
                                        if (parts.length >= 3) {
                                          displayDate = '${int.tryParse(parts[1]) ?? parts[1]}月${int.tryParse(parts[2]) ?? parts[2]}日';
                                        }
                                      }
                                    }
                                    return Padding(
                                      padding: const EdgeInsets.only(top: 8.0),
                                      child: Text(
                                        displayDate,
                                        style: TextStyle(
                                          fontSize: 10,
                                          color: theme.colorScheme.outline,
                                        ),
                                      ),
                                    );
                                  },
                                ),
                              ),
                            ),
                            borderData: FlBorderData(show: false),
                            lineTouchData: LineTouchData(
                              enabled: true,
                              handleBuiltInTouches: false,
                              getTouchedSpotIndicator: (LineChartBarData barData, List<int> spotIndexes) {
                                return spotIndexes.map((spotIndex) {
                                  final spot = barData.spots[spotIndex];
                                  final showDot = spot.y > 0;
                                  return TouchedSpotIndicatorData(
                                    const FlLine(color: Colors.transparent, strokeWidth: 0),
                                    FlDotData(
                                      show: showDot,
                                      getDotPainter: (spot, percent, bar, index) {
                                        return FlDotCirclePainter(
                                          radius: 3.0,
                                          color: bar.color ?? primaryColor,
                                          strokeWidth: 1.5,
                                          strokeColor: isDark ? const Color(0xFF1E2228) : Colors.white,
                                        );
                                      },
                                    ),
                                  );
                                }).toList();
                              },
                              touchTooltipData: LineTouchTooltipData(
                                getTooltipItems: (touchedSpots) => touchedSpots.map((_) => null).toList(),
                              ),
                            ),
                            lineBarsData: lineBarsData,
                          ),
                          duration: Duration.zero,
                        ),

                        // Ultra-smooth, pixel-aligned vertical dashed guideline (0ms latency, zero jaggedness)
                        if (snappedX != null)
                          Positioned(
                            left: snappedX.roundToDouble(),
                            top: 4.0,
                            child: IgnorePointer(
                              child: CustomPaint(
                                size: Size(1.0, totalChartHeight - bottomReserved - 4.0),
                                painter: _SmoothDashedVerticalLinePainter(
                                  color: isDark ? const Color(0x66FFFFFF) : const Color(0x55000000),
                                  strokeWidth: 1.0,
                                  dashHeight: 5.0,
                                  dashSpace: 4.0,
                                ),
                              ),
                            ),
                          ),

                        // Floating Material 3 Tooltip Card positioned beside the dashed line
                        if (snappedX != null && hoveredBucket != null)
                          _buildFloatingTooltip(
                            snappedX: snappedX,
                            totalWidth: totalWidth,
                            bucket: hoveredBucket,
                            sortedModels: sortedModels,
                            modelValues: modelValues,
                            modelColors: modelColors,
                            isDark: isDark,
                            theme: theme,
                            themeProvider: themeProvider,
                          ),
                      ],
                    ),
                  ),
                );
              },
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildFloatingTooltip({
    required double snappedX,
    required double totalWidth,
    required TrendBucket bucket,
    required List<String> sortedModels,
    required Map<String, List<int>> modelValues,
    required Map<String, Color> modelColors,
    required bool isDark,
    required ThemeData theme,
    required ThemeProvider themeProvider,
  }) {
    const tooltipWidth = 260.0;
    // Smart flip: if close to right edge, show on left side of line
    double tooltipLeft = snappedX + 14.0;
    if (tooltipLeft + tooltipWidth > totalWidth) {
      tooltipLeft = snappedX - tooltipWidth - 14.0;
    }
    if (tooltipLeft < 8.0) tooltipLeft = 8.0;

    final tooltipTop = (_mouseY - 50.0).clamp(6.0, 80.0);

    // Format display date
    String displayDate = bucket.date;
    try {
      final parsed = DateTime.parse(bucket.date);
      displayDate = '${parsed.year}年${parsed.month}月${parsed.day}日';
    } catch (_) {}

    // Collect active models on this day
    final idx = _hoveredIndex ?? 0;
    final List<MapEntry<String, int>> activeToday = [];
    for (final model in sortedModels) {
      if (_hiddenModels.contains(model)) continue;
      final t = (modelValues[model]?.length ?? 0) > idx ? modelValues[model]![idx] : 0;
      if (t > 0) {
        activeToday.add(MapEntry(model, t));
      }
    }
    activeToday.sort((a, b) => b.value.compareTo(a.value));

    return Positioned(
      left: tooltipLeft,
      top: tooltipTop,
      child: IgnorePointer(
        child: Container(
          width: tooltipWidth,
          padding: const EdgeInsets.symmetric(horizontal: 12.0, vertical: 10.0),
          decoration: BoxDecoration(
            color: isDark ? const Color(0xFF1E2228) : Colors.white,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(
              color: isDark ? const Color(0xFF333A44) : const Color(0xFFE2E8F0),
            ),
            boxShadow: [
              BoxShadow(
                color: Colors.black.withValues(alpha: isDark ? 0.45 : 0.12),
                blurRadius: 16,
                offset: const Offset(0, 4),
              ),
            ],
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              // Header Row: Date & Day Total
              Row(
                children: [
                  Expanded(
                    child: Text(
                      displayDate,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: 11,
                        fontWeight: FontWeight.bold,
                        color: isDark ? Colors.white : Colors.black87,
                      ),
                    ),
                  ),
                  const SizedBox(width: 8),
                  Text(
                    '${themeProvider.formatTokens(bucket.tokens)} Tokens',
                    style: TextStyle(
                      fontSize: 11,
                      fontWeight: FontWeight.bold,
                      color: theme.colorScheme.primary,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 6),
              Divider(
                height: 1,
                thickness: 0.6,
                color: isDark ? Colors.white12 : Colors.black12,
              ),
              const SizedBox(height: 6),

              // Tabular list of active models
              if (activeToday.isEmpty)
                Text(
                  '当日无用量记录',
                  style: TextStyle(fontSize: 11, color: theme.colorScheme.outline),
                )
              else
                ...activeToday.take(6).map((entry) {
                  final model = entry.key;
                  final tokens = entry.value;
                  final color = modelColors[model] ?? theme.colorScheme.primary;

                  return Padding(
                    padding: const EdgeInsets.symmetric(vertical: 2.0),
                    child: Row(
                      children: [
                        Container(
                          width: 6,
                          height: 6,
                          decoration: BoxDecoration(
                            shape: BoxShape.circle,
                            color: color,
                          ),
                        ),
                        const SizedBox(width: 6),
                        Expanded(
                          child: Text(
                            model,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontSize: 11,
                              color: isDark ? Colors.white70 : Colors.black87,
                            ),
                          ),
                        ),
                        const SizedBox(width: 8),
                        Text(
                          themeProvider.formatTokens(tokens),
                          style: TextStyle(
                            fontSize: 11,
                            fontWeight: FontWeight.w600,
                            color: isDark ? Colors.white : Colors.black,
                          ),
                        ),
                      ],
                    ),
                  );
                }),

              if (activeToday.length > 6) ...[
                const SizedBox(height: 4),
                Text(
                  '...其余 ${activeToday.length - 6} 个模型',
                  style: TextStyle(fontSize: 10, color: theme.colorScheme.outline),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildRangePill({
    required String label,
    required String value,
    required bool isDark,
    required ThemeData theme,
  }) {
    final isSelected = _selectedDays == value;
    return GestureDetector(
      onTap: () {
        if (!isSelected) {
          setState(() {
            _selectedDays = value;
            _hoveredIndex = null;
          });
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
                    color: Colors.black.withValues(alpha: isDark ? 0.25 : 0.08),
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
}

/// Crisp, pixel-aligned dashed vertical guideline with anti-aliasing and rounded micro-dashes
class _SmoothDashedVerticalLinePainter extends CustomPainter {
  final Color color;
  final double strokeWidth;
  final double dashHeight;
  final double dashSpace;

  const _SmoothDashedVerticalLinePainter({
    required this.color,
    this.strokeWidth = 1.0,
    this.dashHeight = 5.0,
    this.dashSpace = 4.0,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..strokeWidth = strokeWidth
      ..strokeCap = StrokeCap.round
      ..isAntiAlias = true
      ..style = PaintingStyle.stroke;

    double y = dashHeight / 2;
    while (y < size.height) {
      final nextY = math.min(y + dashHeight, size.height - dashHeight / 2);
      if (nextY > y) {
        canvas.drawLine(Offset(0, y), Offset(0, nextY), paint);
      }
      y += dashHeight + dashSpace;
    }
  }

  @override
  bool shouldRepaint(_SmoothDashedVerticalLinePainter oldDelegate) {
    return oldDelegate.color != color ||
        oldDelegate.strokeWidth != strokeWidth ||
        oldDelegate.dashHeight != dashHeight ||
        oldDelegate.dashSpace != dashSpace;
  }
}
