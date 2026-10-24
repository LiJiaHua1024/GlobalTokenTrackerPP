import 'dart:async';
import 'dart:math' as math;
import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';
import '../core/theme.dart';

class TokenRateDashboard extends StatefulWidget {
  final TokenRateOverview initialRates;
  final List<String>? filterApps;
  final List<String>? filterModels;
  final VoidCallback? onDataChanged;

  const TokenRateDashboard({
    super.key,
    required this.initialRates,
    this.filterApps,
    this.filterModels,
    this.onDataChanged,
  });

  @override
  State<TokenRateDashboard> createState() => _TokenRateDashboardState();
}

class _TokenRateDashboardState extends State<TokenRateDashboard>
    with SingleTickerProviderStateMixin {
  late TokenRateOverview _rates;
  String _selectedWindow = '1h'; // '1m' | '5m' | '15m' | '1h' | '24h'
  bool _isLiveMonitoring = true;
  bool _isRefreshing = false;
  Timer? _pollingTimer;

  late final AnimationController _pulseController;
  late final Animation<double> _pulseAnimation;

  @override
  void initState() {
    super.initState();
    _rates = widget.initialRates;

    _pulseController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 1200),
    )..repeat(reverse: true);

    _pulseAnimation = Tween<double>(begin: 0.4, end: 1.0).animate(
      CurvedAnimation(parent: _pulseController, curve: Curves.easeInOut),
    );

    // Live polling is on by default, matching the rate page.
    _startPolling();
  }

  @override
  void didUpdateWidget(covariant TokenRateDashboard oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!_isLiveMonitoring) {
      _rates = widget.initialRates;
    }
  }

  @override
  void dispose() {
    _pollingTimer?.cancel();
    _pulseController.dispose();
    super.dispose();
  }

  void _startPolling() {
    _pollingTimer?.cancel();
    // Poll every 3 seconds: run scan + refresh rates
    _pollingTimer = Timer.periodic(const Duration(seconds: 3), (_) {
      _refreshRates(silent: true);
    });
    _refreshRates(silent: true);
  }

  void _toggleLiveMonitoring() {
    setState(() {
      _isLiveMonitoring = !_isLiveMonitoring;
    });

    if (_isLiveMonitoring) {
      _startPolling();
    } else {
      _pollingTimer?.cancel();
    }
  }

  Future<void> _refreshRates({bool silent = false}) async {
    // Silent timer ticks must also respect the in-flight guard: a scan can
    // easily outlive the 3s poll interval and pile up behind the engine lock.
    if (_isRefreshing) return;
    _isRefreshing = true;
    if (!silent && mounted) setState(() {});

    try {
      if (_isLiveMonitoring) {
        // Quick background log check
        try {
          await FfiBridge.instance.scan();
        } catch (_) {}
      }

      final rates = await FfiBridge.instance.getTokenRates(
        filterApps: widget.filterApps,
        filterModels: widget.filterModels,
      );

      if (mounted) {
        setState(() {
          _rates = rates;
          _isRefreshing = false;
        });
        widget.onDataChanged?.call();
      } else {
        _isRefreshing = false;
      }
    } catch (_) {
      if (mounted) {
        setState(() => _isRefreshing = false);
      } else {
        _isRefreshing = false;
      }
    }
  }

  RateMetric _getCurrentMetric() {
    switch (_selectedWindow) {
      case '1m':
        return _rates.m1;
      case '5m':
        return _rates.m5;
      case '15m':
        return _rates.m15;
      case '24h':
        return _rates.h24;
      case '1h':
      default:
        return _rates.h1;
    }
  }

  String _formatRelativeTime(int? timestampMs) {
    if (timestampMs == null || timestampMs <= 0) return '暂无记录';
    final now = DateTime.now().millisecondsSinceEpoch;
    final diffMs = now - timestampMs;
    if (diffMs < 0) return '刚刚';
    final diffSec = diffMs ~/ 1000;
    if (diffSec < 60) return '$diffSec 秒前';
    final diffMin = diffSec ~/ 60;
    if (diffMin < 60) return '$diffMin 分钟前';
    final diffHour = diffMin ~/ 60;
    if (diffHour < 24) return '$diffHour 小时前';
    final diffDays = diffHour ~/ 24;
    return '$diffDays 天前';
  }

  String _formatNumberCompact(double val) {
    if (val >= 1e9) {
      return '${(val / 1e9).toStringAsFixed(1)}B';
    } else if (val >= 1e6) {
      return '${(val / 1e6).toStringAsFixed(1)}M';
    } else if (val >= 1e3) {
      return '${(val / 1e3).toStringAsFixed(1)}k';
    }
    return val >= 10 ? val.toStringAsFixed(0) : val.toStringAsFixed(1);
  }

  String _formatTps(double tps) {
    if (tps >= 1000) {
      return '${(tps / 1000).toStringAsFixed(1)}k';
    }
    return tps.toStringAsFixed(1);
  }

  String _formatTtft(double? ms) {
    if (ms == null) return '—';
    if (ms >= 1000) {
      final s = ms / 1000;
      return '${s >= 10 ? s.toStringAsFixed(0) : s.toStringAsFixed(1)} s';
    }
    return '${ms.round()} ms';
  }

  /// `≈` prefix marks timing that includes timestamp-estimated spans.
  String _formatTpsApprox(double? tps, {required bool estimated}) {
    if (tps == null) return '—';
    return '${estimated ? '≈' : ''}${_formatTps(tps)} tok/s';
  }

  /// Full speed breakdown for a model row tooltip — same figures the rate
  /// page shows.
  String _speedTooltip(ModelRateItem m) {
    final lines = <String>[];
    final estimated = m.speed.speedEstMs > 0;
    if (m.speed.speedMs > 0) {
      final total = (m.speed.speedIn + m.speed.speedOut) / m.speed.speedMs * 1000;
      final input = m.speed.speedIn / m.speed.speedMs * 1000;
      lines.add('总 ${_formatTpsApprox(total, estimated: estimated)}');
      lines.add('输入 ${_formatTpsApprox(input, estimated: estimated)}');
      lines.add('输出 ${_formatTpsApprox(m.outputTps, estimated: estimated)}');
    }
    if (m.speed.decodeMs > 0) {
      lines.add('解码 ${_formatTps(m.decodeTps ?? 0)} tok/s');
    }
    if (m.speed.ttftEvents > 0) {
      lines.add('预填 ${_formatTps(m.prefillTps ?? 0)} tok/s');
      lines.add('平均 TTFT ${_formatTtft(m.avgTtftMs)}');
    }
    if (m.speed.speedMs > 0) {
      final tail = estimated
          ? ' · 估算 ${(m.speed.speedEstMs / m.speed.speedMs * 100).toStringAsFixed(0)}%'
          : '';
      lines.add(
          '${m.speed.timedEvents} 次计时请求 · 有效 ${(m.speed.speedMs / 1000).toStringAsFixed(1)} 秒$tail');
    }
    return lines.isEmpty ? '暂无计时数据（速度统计需请求时长或首尾时间戳）' : lines.join('\n');
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final themeProvider = Provider.of<ThemeProvider>(context);
    final metric = _getCurrentMetric();

    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.4),
          width: 1,
        ),
      ),
      color: theme.colorScheme.surfaceContainerHighest.withValues(alpha: 0.3),
      child: Padding(
        padding: const EdgeInsets.all(20.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Top Header: Title, Status, Live Toggle & Manual Refresh
            Row(
              children: [
                Container(
                  padding: const EdgeInsets.all(8),
                  decoration: BoxDecoration(
                    color: theme.colorScheme.primary.withValues(alpha: 0.12),
                    borderRadius: BorderRadius.circular(10),
                  ),
                  child: Icon(
                    Icons.speed_rounded,
                    color: theme.colorScheme.primary,
                    size: 22,
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Wrap(
                    crossAxisAlignment: WrapCrossAlignment.center,
                    spacing: 8,
                    runSpacing: 4,
                    children: [
                      Text(
                        'Token 增长速率 · 并发实时监控',
                        style: theme.textTheme.titleMedium?.copyWith(
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                      // Active / Idle status badge
                      if (_rates.isActive)
                        FadeTransition(
                          opacity: _pulseAnimation,
                          child: Container(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 8,
                              vertical: 3,
                            ),
                            decoration: BoxDecoration(
                              color: const Color(0xFF34A853)
                                  .withValues(alpha: 0.15),
                              borderRadius: BorderRadius.circular(12),
                              border: Border.all(
                                color: const Color(0xFF34A853)
                                    .withValues(alpha: 0.4),
                              ),
                            ),
                            child: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                Container(
                                  width: 7,
                                  height: 7,
                                  decoration: const BoxDecoration(
                                    color: Color(0xFF34A853),
                                    shape: BoxShape.circle,
                                  ),
                                ),
                                const SizedBox(width: 5),
                                const Text(
                                  '并发活跃中',
                                  style: TextStyle(
                                    color: Color(0xFF34A853),
                                    fontSize: 12,
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        )
                      else
                        Container(
                          padding: const EdgeInsets.symmetric(
                            horizontal: 8,
                            vertical: 3,
                          ),
                          decoration: BoxDecoration(
                            color: theme.colorScheme.outlineVariant
                                .withValues(alpha: 0.2),
                            borderRadius: BorderRadius.circular(12),
                          ),
                          child: Text(
                            '当前空闲 · 最近活跃: ${_formatRelativeTime(_rates.latestEventMs)}',
                            style: TextStyle(
                              color: theme.colorScheme.outline,
                              fontSize: 12,
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
                // Live monitoring toggle
                FilterChip(
                  avatar: _isLiveMonitoring
                      ? FadeTransition(
                          opacity: _pulseAnimation,
                          child: Container(
                            width: 8,
                            height: 8,
                            decoration: const BoxDecoration(
                              color: Color(0xFF34A853),
                              shape: BoxShape.circle,
                            ),
                          ),
                        )
                      : null,
                  label: Text(_isLiveMonitoring ? '实时轮询中 (3s)' : '开启实时监控'),
                  selected: _isLiveMonitoring,
                  onSelected: (_) => _toggleLiveMonitoring(),
                  visualDensity: VisualDensity.compact,
                ),
                const SizedBox(width: 8),
                IconButton.filledTonal(
                  onPressed: _isRefreshing ? null : () => _refreshRates(),
                  icon: _isRefreshing
                      ? const SizedBox(
                          width: 16,
                          height: 16,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Icon(Icons.refresh, size: 18),
                  tooltip: '刷新速率数据',
                  visualDensity: VisualDensity.compact,
                ),
              ],
            ),
            const SizedBox(height: 16),

            // Time Window Segmented Tabs
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: SegmentedButton<String>(
                segments: const [
                  ButtonSegment(value: '1m', label: Text('近 1 分钟')),
                  ButtonSegment(value: '5m', label: Text('近 5 分钟')),
                  ButtonSegment(value: '15m', label: Text('近 15 分钟')),
                  ButtonSegment(value: '1h', label: Text('近 1 小时')),
                  ButtonSegment(value: '24h', label: Text('近 24 小时')),
                ],
                selected: {_selectedWindow},
                onSelectionChanged: (val) {
                  setState(() {
                    _selectedWindow = val.first;
                  });
                },
                style: const ButtonStyle(
                  visualDensity: VisualDensity.compact,
                  tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                ),
              ),
            ),
            const SizedBox(height: 16),

            // Rate KPI Cards
            LayoutBuilder(builder: (context, constraints) {
              final cols = constraints.maxWidth > 900
                  ? 3
                  : (constraints.maxWidth > 520 ? 2 : 1);
              // Fixed height from content (this card is one notch more
              // compact than the rate page's) — keeps cards from growing
              // tall and empty on wide windows.
              final cardWidth = (constraints.maxWidth - 12.0 * (cols - 1)) / cols;
              return GridView.count(
                crossAxisCount: cols,
                crossAxisSpacing: 12,
                mainAxisSpacing: 12,
                shrinkWrap: true,
                physics: const NeverScrollableScrollPhysics(),
                childAspectRatio: cardWidth / 110,
                children: [
                  // KPI 1: Overall Token Velocity
                  _buildKpiCard(
                    context,
                    title: 'Token 增长速率',
                    value:
                        '${themeProvider.formatTokens(metric.tokensPerMin.round())} /min',
                    subtitle:
                        '${_formatTps(metric.tokensPerSec)} tok/s · 增量 ${themeProvider.formatTokens(metric.totalTokens)}',
                    icon: Icons.trending_up_rounded,
                    accentColor: const Color(0xFF1A73E8),
                  ),
                  // KPI 2: Output speed (request-level, timed events only)
                  _buildKpiCard(
                    context,
                    title: '输出速度',
                    value: _formatTpsApprox(metric.outputTps,
                        estimated: metric.speed.speedEstMs > 0),
                    subtitle:
                        '解码 ${metric.decodeTps != null ? '${_formatTps(metric.decodeTps!)} tok/s' : '—'} · ${metric.speed.timedEvents} 次请求',
                    icon: Icons.speed_rounded,
                    accentColor: const Color(0xFFEA4335),
                  ),
                  // KPI 3: Mean TTFT
                  _buildKpiCard(
                    context,
                    title: '平均 TTFT',
                    value: _formatTtft(metric.avgTtftMs),
                    subtitle:
                        '预填 ${metric.prefillTps != null ? '${_formatTps(metric.prefillTps!)} tok/s' : '—'} · ${metric.speed.ttftEvents} 次请求',
                    icon: Icons.hourglass_top_rounded,
                    accentColor: const Color(0xFF12B5CB),
                  ),
                  // KPI 4: Input / Output Split
                  _buildKpiCard(
                    context,
                    title: '输入 / 输出 速率分布',
                    value:
                        '入 ${themeProvider.formatTokens(metric.inputTokensPerMin.round())} · 出 ${themeProvider.formatTokens(metric.outputTokensPerMin.round())} /min',
                    subtitle:
                        '缓存命中: ${themeProvider.formatTokens(metric.cacheReadTokens)} · 写入: ${themeProvider.formatTokens(metric.cacheWriteTokens)}',
                    icon: Icons.sync_alt_rounded,
                    accentColor: const Color(0xFF8E24AA),
                  ),
                  // KPI 5: Burn Rate (USD / hour)
                  _buildKpiCard(
                    context,
                    title: '预估费用增速',
                    value: '\$${metric.costPerHour.toStringAsFixed(2)} /h',
                    subtitle: '该窗口实际花费: \$${metric.costUsd.toStringAsFixed(4)}',
                    icon: Icons.local_fire_department_rounded,
                    accentColor: const Color(0xFF34A853),
                  ),
                  // KPI 6: Concurrency & Peak Burst
                  _buildKpiCard(
                    context,
                    title: '并发请求数与峰值',
                    value:
                        '${metric.requestsPerMin.toStringAsFixed(1)} req/min',
                    subtitle:
                        '1h 峰值: ${themeProvider.formatTokens(_rates.peak1mIn1h.tokensPerMin.round())}/min (${_formatTps(_rates.peak1mIn1h.tokensPerSec)} TPS)',
                    icon: Icons.bolt_rounded,
                    accentColor: const Color(0xFFFBBC05),
                  ),
                ],
              );
            }),
            const SizedBox(height: 20),

            // Timeline Chart (Past 1 Hour minute-by-minute)
            _buildTimelineChart(context),
            const SizedBox(height: 20),

            // Model Breakdown Table/List (if any)
            if (_rates.topModels1h.isNotEmpty) ...[
              Text(
                '高频并发模型排行',
                style: theme.textTheme.titleSmall?.copyWith(
                  fontWeight: FontWeight.bold,
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: 10),
              _buildTopModelsList(context, themeProvider),
            ],
          ],
        ),
      ),
    );
  }

  Widget _buildKpiCard(
    BuildContext context, {
    required String title,
    required String value,
    required String subtitle,
    required IconData icon,
    required Color accentColor,
  }) {
    final theme = Theme.of(context);
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: theme.colorScheme.surface,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.35),
          width: 1,
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Expanded(
                child: Text(
                  title,
                  style: theme.textTheme.labelMedium?.copyWith(
                    color: theme.colorScheme.outline,
                    fontWeight: FontWeight.w500,
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              Container(
                padding: const EdgeInsets.all(4),
                decoration: BoxDecoration(
                  color: accentColor.withValues(alpha: 0.12),
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Icon(icon, size: 16, color: accentColor),
              ),
            ],
          ),
          const SizedBox(height: 4),
          Text(
            value,
            style: theme.textTheme.titleLarge?.copyWith(
              fontWeight: FontWeight.bold,
              color: accentColor,
              letterSpacing: -0.5,
            ),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
          const SizedBox(height: 4),
          Text(
            subtitle,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
              fontSize: 11,
            ),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
        ],
      ),
    );
  }

  Widget _buildTimelineChart(BuildContext context) {
    final theme = Theme.of(context);
    final themeProvider = Provider.of<ThemeProvider>(context);
    final timeline = _rates.timeline1h;

    // Check if timeline has any non-zero points
    final hasData = timeline.any((b) => b.totalTokens > 0);
    final maxTok = timeline.isEmpty
        ? 0
        : timeline.map((b) => b.totalTokens).reduce(math.max);

    // Compute Y-axis ceiling with headroom
    final maxY = hasData ? (maxTok * 1.25).toDouble() : 1000.0;

    return Container(
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: theme.colorScheme.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.35),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                '近 1 小时分钟级速率走势',
                style: theme.textTheme.titleSmall?.copyWith(
                  fontWeight: FontWeight.bold,
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
              if (hasData)
                Text(
                  '峰值: ${themeProvider.formatTokens(_rates.peak1mIn1h.tokensPerMin.round())}/min',
                  style: theme.textTheme.labelSmall?.copyWith(
                    color: const Color(0xFF1A73E8),
                    fontWeight: FontWeight.w600,
                  ),
                ),
            ],
          ),
          const SizedBox(height: 16),
          SizedBox(
            height: 180,
            child: LineChart(
              LineChartData(
                minX: 0,
                maxX: (timeline.length - 1).toDouble().clamp(0, 59),
                minY: 0,
                maxY: maxY,
                gridData: FlGridData(
                  show: true,
                  drawVerticalLine: false,
                  horizontalInterval: maxY / 4,
                  getDrawingHorizontalLine: (value) => FlLine(
                    color:
                        theme.colorScheme.outlineVariant.withValues(alpha: 0.2),
                    strokeWidth: 1,
                    dashArray: [4, 4],
                  ),
                ),
                titlesData: FlTitlesData(
                  rightTitles: const AxisTitles(
                      sideTitles: SideTitles(showTitles: false)),
                  topTitles: const AxisTitles(
                      sideTitles: SideTitles(showTitles: false)),
                  leftTitles: AxisTitles(
                    sideTitles: SideTitles(
                      showTitles: true,
                      reservedSize: 45,
                      interval: maxY / 4,
                      getTitlesWidget: (value, meta) {
                        if (value == 0) return const SizedBox.shrink();
                        return Text(
                          _formatNumberCompact(value),
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
                      reservedSize: 22,
                      interval: 10, // every 10 minutes
                      getTitlesWidget: (value, meta) {
                        final idx = value.toInt();
                        if (idx >= 0 && idx < timeline.length) {
                          return Text(
                            timeline[idx].label,
                            style: TextStyle(
                              fontSize: 10,
                              color: theme.colorScheme.outline,
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
                  touchTooltipData: LineTouchTooltipData(
                    getTooltipColor: (_) => theme.colorScheme.inverseSurface,
                    getTooltipItems: (touchedSpots) {
                      return touchedSpots.map((spot) {
                        final idx = spot.x.toInt();
                        if (idx < 0 || idx >= timeline.length) return null;
                        final b = timeline[idx];
                        return LineTooltipItem(
                          '${b.label}\n'
                          '${themeProvider.formatTokens(b.totalTokens)} tokens/min\n'
                          '${_formatTps(b.tokensPerSec)} TPS · ${b.events} 请求\n'
                          '费用: \$${b.costUsd.toStringAsFixed(4)}',
                          TextStyle(
                            color: theme.colorScheme.onInverseSurface,
                            fontSize: 11,
                            fontWeight: FontWeight.w500,
                          ),
                        );
                      }).toList();
                    },
                  ),
                ),
                lineBarsData: [
                  LineChartBarData(
                    spots: timeline.asMap().entries.map((e) {
                      return FlSpot(
                          e.key.toDouble(), e.value.totalTokens.toDouble());
                    }).toList(),
                    isCurved: true,
                    curveSmoothness: 0.25,
                    color: const Color(0xFF1A73E8),
                    barWidth: 2.5,
                    isStrokeCapRound: true,
                    dotData: const FlDotData(show: false),
                    belowBarData: BarAreaData(
                      show: true,
                      gradient: LinearGradient(
                        begin: Alignment.topCenter,
                        end: Alignment.bottomCenter,
                        colors: [
                          const Color(0xFF1A73E8).withValues(alpha: 0.35),
                          const Color(0xFF1A73E8).withValues(alpha: 0.0),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildTopModelsList(
      BuildContext context, ThemeProvider themeProvider) {
    final theme = Theme.of(context);
    final models = _rates.topModels1h.take(6).toList();

    return Column(
      children: models.map((m) {
        return Tooltip(
          message: _speedTooltip(m),
          waitDuration: const Duration(milliseconds: 400),
          child: Container(
            margin: const EdgeInsets.only(bottom: 8),
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
            decoration: BoxDecoration(
              color: theme.colorScheme.surface,
              borderRadius: BorderRadius.circular(10),
              border: Border.all(
                color: theme.colorScheme.outlineVariant.withValues(alpha: 0.25),
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    // App badge
                    Container(
                      padding:
                          const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                      decoration: BoxDecoration(
                        color: theme.colorScheme.primary.withValues(alpha: 0.1),
                        borderRadius: BorderRadius.circular(4),
                      ),
                      child: Text(
                        m.app,
                        style: TextStyle(
                          fontSize: 11,
                          color: theme.colorScheme.primary,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                    const SizedBox(width: 8),
                    // Model name
                    Expanded(
                      child: Text(
                        m.model,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          fontWeight: FontWeight.bold,
                        ),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                    // Rate
                    Text(
                      '${themeProvider.formatTokens(m.tokensPerMin.round())} /min',
                      style: TextStyle(
                        fontWeight: FontWeight.bold,
                        color: theme.colorScheme.primary,
                      ),
                    ),
                  ],
                ),
                if (m.outputTps != null || m.avgTtftMs != null) ...[
                  const SizedBox(height: 6),
                  Text(
                    [
                      if (m.outputTps != null)
                        '输出 ${_formatTpsApprox(m.outputTps, estimated: m.speed.speedEstMs > 0)}',
                      if (m.decodeTps != null)
                        '解码 ${_formatTps(m.decodeTps!)} tok/s',
                      if (m.avgTtftMs != null)
                        'TTFT ${_formatTtft(m.avgTtftMs)}',
                    ].join(' · '),
                    style: TextStyle(
                      fontSize: 11,
                      color: theme.colorScheme.outline,
                    ),
                  ),
                ],
                const SizedBox(height: 6),
                // Progress Bar + percentage & cost
                Row(
                  children: [
                    Expanded(
                      child: ClipRRect(
                        borderRadius: BorderRadius.circular(4),
                        child: LinearProgressIndicator(
                          value: (m.percentage / 100.0).clamp(0.0, 1.0),
                          minHeight: 6,
                          backgroundColor: theme
                              .colorScheme.surfaceContainerHighest
                              .withValues(alpha: 0.5),
                          valueColor: AlwaysStoppedAnimation<Color>(
                            theme.colorScheme.primary,
                          ),
                        ),
                      ),
                    ),
                    const SizedBox(width: 12),
                    Text(
                      '${m.percentage.toStringAsFixed(1)}%',
                      style: theme.textTheme.bodySmall?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(width: 12),
                    Text(
                      '\$${m.costUsd.toStringAsFixed(2)}',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: const Color(0xFF34A853),
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        );
      }).toList(),
    );
  }
}
