import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

enum ChartDisplayMode {
  pie,
  donut,
}

class PieDonutChartItem {
  final String label;
  final double value;
  final Color? color;
  final String? subtitle;

  const PieDonutChartItem({
    required this.label,
    required this.value,
    this.color,
    this.subtitle,
  });
}

class PieDonutChart extends StatefulWidget {
  final String title;
  final List<PieDonutChartItem> items;
  final String Function(double)? valueFormatter;
  final bool initialIsDonut;
  final String centerUnit;

  const PieDonutChart({
    super.key,
    required this.title,
    required this.items,
    this.valueFormatter,
    this.initialIsDonut = false,
    this.centerUnit = '',
  });

  @override
  State<PieDonutChart> createState() => _PieDonutChartState();
}

class _PieDonutChartState extends State<PieDonutChart> {
  int _touchedIndex = -1;
  late ChartDisplayMode _mode;

  static const List<Color> _palette = [
    Color(0xFF1A73E8), // Google Blue
    Color(0xFF34A853), // Google Green
    Color(0xFFFBBC05), // Google Yellow
    Color(0xFFEA4335), // Google Red
    Color(0xFF8E24AA), // Purple
    Color(0xFF00ACC1), // Cyan
    Color(0xFFFF7043), // Deep Orange
    Color(0xFF5C6BC0), // Indigo
    Color(0xFF26A69A), // Teal
    Color(0xFF78909C), // Blue Grey
  ];

  @override
  void initState() {
    super.initState();
    _mode = widget.initialIsDonut ? ChartDisplayMode.donut : ChartDisplayMode.pie;
  }

  String _formatValue(double val) {
    if (widget.valueFormatter != null) {
      return widget.valueFormatter!(val);
    }
    final absV = val.abs();
    if (absV >= 1000000000000) {
      return '${(val / 1000000000000).toStringAsFixed(2)}T';
    } else if (absV >= 1000000000) {
      return '${(val / 1000000000).toStringAsFixed(2)}B';
    } else if (absV >= 1000000) {
      return '${(val / 1000000).toStringAsFixed(2)}M';
    } else if (absV >= 1000) {
      return '${(val / 1000).toStringAsFixed(1)}K';
    }
    return NumberFormat('#,##0.##').format(val);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final total = widget.items.fold<double>(0.0, (sum, item) => sum + item.value);

    // Filter out slices that are zero or negligible (< 0.05% of total)
    final validItems = widget.items.where((i) {
      if (i.value <= 0.0001) return false;
      if (total > 0 && (i.value / total * 100) < 0.05) return false;
      return true;
    }).toList();

    if (validItems.isEmpty || total <= 0) {
      return Card(
        child: Container(
          height: 320,
          padding: const EdgeInsets.all(20),
          alignment: Alignment.center,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(Icons.pie_chart_outline, size: 48, color: theme.colorScheme.outline),
              const SizedBox(height: 12),
              Text(widget.title, style: theme.textTheme.titleMedium),
              const SizedBox(height: 4),
              Text("暂无数据", style: theme.textTheme.bodyMedium?.copyWith(color: theme.colorScheme.outline)),
            ],
          ),
        ),
      );
    }

    final hasHover = _touchedIndex >= 0 && _touchedIndex < validItems.length;
    final hoveredItem = hasHover ? validItems[_touchedIndex] : null;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Header with Title & Mode Toggle + Dynamic Selected Item Info
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        widget.title,
                        style: theme.textTheme.titleMedium?.copyWith(
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                      const SizedBox(height: 4),
                      Text(
                        hasHover
                            ? '已选: ${hoveredItem!.label} (${_formatValue(hoveredItem.value)} · ${(hoveredItem.value / total * 100).toStringAsFixed(1)}%)'
                            : '总计: ${_formatValue(total)} ${widget.centerUnit}',
                        style: TextStyle(
                          fontSize: 12,
                          color: hasHover ? theme.colorScheme.primary : theme.colorScheme.outline,
                          fontWeight: hasHover ? FontWeight.bold : FontWeight.normal,
                        ),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                // Toggle Button: Pie vs Donut
                SegmentedButton<ChartDisplayMode>(
                  style: const ButtonStyle(
                    visualDensity: VisualDensity.compact,
                  ),
                  segments: const [
                    ButtonSegment<ChartDisplayMode>(
                      value: ChartDisplayMode.pie,
                      icon: Icon(Icons.pie_chart, size: 18),
                      tooltip: '实心饼图',
                    ),
                    ButtonSegment<ChartDisplayMode>(
                      value: ChartDisplayMode.donut,
                      icon: Icon(Icons.donut_large, size: 18),
                      tooltip: '环形图',
                    ),
                  ],
                  selected: {_mode},
                  onSelectionChanged: (newSelection) {
                    setState(() {
                      _mode = newSelection.first;
                    });
                  },
                ),
              ],
            ),
            const SizedBox(height: 16),

            // Chart Core Canvas
            SizedBox(
              height: 200,
              child: Stack(
                alignment: Alignment.center,
                children: [
                  PieChart(
                    PieChartData(
                      pieTouchData: PieTouchData(
                        touchCallback: (FlTouchEvent event, pieTouchResponse) {
                          final newIndex = (!event.isInterestedForInteractions ||
                                  pieTouchResponse == null ||
                                  pieTouchResponse.touchedSection == null)
                              ? -1
                              : pieTouchResponse.touchedSection!.touchedSectionIndex;
                          if (_touchedIndex != newIndex) {
                            setState(() {
                              _touchedIndex = newIndex;
                            });
                          }
                        },
                      ),
                      startDegreeOffset: -90,
                      borderData: FlBorderData(show: false),
                      sectionsSpace: 2,
                      centerSpaceRadius: _mode == ChartDisplayMode.donut ? 50 : 0,
                      sections: _generateSections(validItems, total, theme),
                    ),
                    duration: const Duration(milliseconds: 350),
                    curve: Curves.easeInOutCubic,
                  ),

                  // Center info indicator when in Donut mode (Constrained to 92px width to never overlap the ring)
                  if (_mode == ChartDisplayMode.donut)
                    SizedBox(
                      width: 92,
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          FittedBox(
                            fit: BoxFit.scaleDown,
                            child: Text(
                              hasHover
                                  ? _formatValue(hoveredItem!.value)
                                  : _formatValue(total),
                              style: theme.textTheme.titleMedium?.copyWith(
                                fontWeight: FontWeight.bold,
                                color: theme.colorScheme.primary,
                              ),
                            ),
                          ),
                          const SizedBox(height: 2),
                          Text(
                            hasHover
                                ? '占比 ${(hoveredItem!.value / total * 100).toStringAsFixed(1)}%'
                                : '总计 ${widget.centerUnit}',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            textAlign: TextAlign.center,
                            style: theme.textTheme.bodySmall?.copyWith(
                              color: theme.colorScheme.outline,
                              fontSize: 11,
                              fontWeight: FontWeight.w500,
                            ),
                          ),
                        ],
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(height: 16),

            // Interactive Legend Chips
            Wrap(
              spacing: 8,
              runSpacing: 6,
              children: List.generate(validItems.length, (i) {
                final item = validItems[i];
                final isSelected = _touchedIndex == i;
                final color = item.color ?? _palette[i % _palette.length];
                final percent = (item.value / total * 100).toStringAsFixed(1);

                return MouseRegion(
                  onEnter: (_) {
                    if (_touchedIndex != i) setState(() => _touchedIndex = i);
                  },
                  onExit: (_) {
                    if (_touchedIndex == i) setState(() => _touchedIndex = -1);
                  },
                  child: FilterChip(
                    avatar: CircleAvatar(
                      backgroundColor: color,
                      radius: 6,
                    ),
                    label: Text(
                      '${item.label} ($percent%)',
                      style: theme.textTheme.bodySmall?.copyWith(
                        fontWeight: isSelected ? FontWeight.bold : FontWeight.normal,
                      ),
                    ),
                    selected: isSelected,
                    onSelected: (_) {
                      setState(() {
                        _touchedIndex = isSelected ? -1 : i;
                      });
                    },
                    visualDensity: VisualDensity.compact,
                  ),
                );
              }),
            ),
          ],
        ),
      ),
    );
  }

  List<PieChartSectionData> _generateSections(
    List<PieDonutChartItem> items,
    double total,
    ThemeData theme,
  ) {
    return List.generate(items.length, (i) {
      final item = items[i];
      final isTouched = i == _touchedIndex;
      final percent = (item.value / total * 100);
      final color = item.color ?? _palette[i % _palette.length];

      // Base radius vs Explode Radius on Hover/Tap
      final baseRadius = _mode == ChartDisplayMode.donut ? 38.0 : 80.0;
      final radius = isTouched ? baseRadius + 10.0 : baseRadius;

      return PieChartSectionData(
        color: color,
        value: item.value,
        title: percent > 6 ? '${percent.toStringAsFixed(0)}%' : '',
        radius: radius,
        titleStyle: TextStyle(
          fontSize: isTouched ? 14 : 12,
          fontWeight: FontWeight.bold,
          color: Colors.white,
          shadows: const [
            Shadow(color: Colors.black38, blurRadius: 4),
          ],
        ),
        badgePositionPercentageOffset: 1.15,
      );
    });
  }
}
