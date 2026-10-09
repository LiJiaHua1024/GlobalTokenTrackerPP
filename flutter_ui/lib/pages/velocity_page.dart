import 'dart:async';
import 'dart:math' as math;
import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';
import '../core/theme.dart';

class VelocityPage extends StatefulWidget {
  final TokenRateOverview? initialRates;
  final List<String>? initialApps;
  final List<String>? initialModels;

  const VelocityPage({
    super.key,
    this.initialRates,
    this.initialApps,
    this.initialModels,
  });

  @override
  State<VelocityPage> createState() => _VelocityPageState();
}

class _VelocityPageState extends State<VelocityPage>
    with SingleTickerProviderStateMixin {
  bool _loading = true;
  bool _isRefreshing = false;
  String? _error;

  TokenRateOverview? _rates;
  List<String> _availableApps = [];
  List<String> _availableModels = [];
  final List<String> _selectedApps = [];
  final List<String> _selectedModels = [];

  String _selectedWindow = '1h'; // '1m' | '5m' | '15m' | '1h' | '24h'
  bool _isLiveMonitoring = false;
  Timer? _pollingTimer;

  late final AnimationController _pulseController;
  late final Animation<double> _pulseAnimation;

  @override
  void initState() {
    super.initState();
    _pulseController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 1200),
    )..repeat(reverse: true);

    _pulseAnimation = Tween<double>(begin: 0.35, end: 1.0).animate(
      CurvedAnimation(parent: _pulseController, curve: Curves.easeInOut),
    );

    if (widget.initialRates != null) {
      _rates = widget.initialRates;
      _availableApps = List.from(widget.initialApps ?? []);
      _availableModels = List.from(widget.initialModels ?? []);
      _loading = false;
    } else {
      _loadInitialData();
    }
  }

  @override
  void dispose() {
    _pollingTimer?.cancel();
    _pulseController.dispose();
    super.dispose();
  }

  Future<void> _loadInitialData() async {
    setState(() {
      _loading = true;
      _error = null;
    });

    try {
      // 1. Fetch overview to get available apps & models for filtering
      final overview = await FfiBridge.instance.getOverview();
      _availableApps = overview.apps;
      _availableModels = overview.models;

      // 2. Fetch token rates
      final rates = await FfiBridge.instance.getTokenRates(
        filterApps: _selectedApps.isNotEmpty ? _selectedApps : null,
        filterModels: _selectedModels.isNotEmpty ? _selectedModels : null,
      );

      if (mounted) {
        setState(() {
          _rates = rates;
          _loading = false;
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _error = e.toString();
          _loading = false;
        });
      }
    }
  }

  void _toggleLiveMonitoring() {
    setState(() {
      _isLiveMonitoring = !_isLiveMonitoring;
    });

    _pollingTimer?.cancel();
    if (_isLiveMonitoring) {
      _pollingTimer = Timer.periodic(const Duration(seconds: 3), (_) {
        _refreshRates(silent: true);
      });
      _refreshRates(silent: true);
    }
  }

  Future<void> _refreshRates({bool silent = false, bool triggerScan = false}) async {
    if (_isRefreshing && !silent) return;
    if (!silent) {
      setState(() => _isRefreshing = true);
    }

    try {
      if (triggerScan || _isLiveMonitoring) {
        try {
          await FfiBridge.instance.scan();
        } catch (_) {}
      }

      final rates = await FfiBridge.instance.getTokenRates(
        filterApps: _selectedApps.isNotEmpty ? _selectedApps : null,
        filterModels: _selectedModels.isNotEmpty ? _selectedModels : null,
      );

      if (mounted) {
        setState(() {
          _rates = rates;
          _isRefreshing = false;
        });
      }
    } catch (e) {
      if (mounted && !silent) {
        setState(() {
          _isRefreshing = false;
          _error = e.toString();
        });
      }
    }
  }

  RateMetric _getCurrentMetric() {
    final r = _rates;
    if (r == null) {
      return RateMetric(
        window: _selectedWindow,
        windowSecs: 60,
        startMs: 0,
        endMs: 0,
        events: 0,
        totalTokens: 0,
        inputTokens: 0,
        outputTokens: 0,
        reasoningTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        costUsd: 0,
        tokensPerMin: 0,
        tokensPerSec: 0,
        inputTokensPerMin: 0,
        outputTokensPerMin: 0,
        costPerHour: 0,
        requestsPerMin: 0,
      );
    }
    switch (_selectedWindow) {
      case '1m':
        return r.m1;
      case '5m':
        return r.m5;
      case '15m':
        return r.m15;
      case '24h':
        return r.h24;
      case '1h':
      default:
        return r.h1;
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

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final themeProvider = Provider.of<ThemeProvider>(context);

    if (_loading && _rates == null) {
      return const Center(child: CircularProgressIndicator());
    }

    if (_error != null && _rates == null) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.error_outline, size: 48, color: theme.colorScheme.error),
            const SizedBox(height: 16),
            Text('加载速率统计失败', style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
            const SizedBox(height: 16),
            FilledButton.icon(
              onPressed: () => _loadInitialData(),
              icon: const Icon(Icons.refresh),
              label: const Text('重试'),
            ),
          ],
        ),
      );
    }

    final rates = _rates!;
    final metric = _getCurrentMetric();

    return RefreshIndicator(
      onRefresh: () => _refreshRates(),
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(24.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Top Header: Page Title, Status Badge, Live Toggle & Action Buttons
            _buildHeaderBar(context, rates),
            const SizedBox(height: 16),

            // Tool Filters (if available)
            if (_availableApps.isNotEmpty) ...[
              _buildFilterChips(context),
              const SizedBox(height: 16),
            ],

            // Window Selection Tabs (Segmented Button)
            _buildWindowSelector(context),
            const SizedBox(height: 16),

            // 4 Hero KPI Cards
            _buildKpiGrid(context, themeProvider, rates, metric),
            const SizedBox(height: 24),

            // 60-Minute Minute-by-Minute Velocity Timeline Chart
            _buildTimelineChart(context, themeProvider, rates),
            const SizedBox(height: 24),

            // Dual Columns: Top Models Breakdown & Top Apps Breakdown
            LayoutBuilder(builder: (context, constraints) {
              final isWide = constraints.maxWidth > 850;
              if (isWide) {
                return Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Expanded(
                      flex: 6,
                      child: _buildTopModelsCard(context, themeProvider, rates),
                    ),
                    const SizedBox(width: 16),
                    Expanded(
                      flex: 4,
                      child: _buildTopAppsCard(context, themeProvider, rates),
                    ),
                  ],
                );
              } else {
                return Column(
                  children: [
                    _buildTopModelsCard(context, themeProvider, rates),
                    const SizedBox(height: 16),
                    _buildTopAppsCard(context, themeProvider, rates),
                  ],
                );
              }
            }),
            const SizedBox(height: 24),

            // Burst Insights & Peak Records
            _buildBurstInsightsCard(context, themeProvider, rates),
          ],
        ),
      ),
    );
  }

  Widget _buildHeaderBar(BuildContext context, TokenRateOverview rates) {
    final theme = Theme.of(context);
    return Row(
      children: [
        Container(
          padding: const EdgeInsets.all(10),
          decoration: BoxDecoration(
            color: theme.colorScheme.primary.withValues(alpha: 0.12),
            borderRadius: BorderRadius.circular(12),
          ),
          child: Icon(
            Icons.speed_rounded,
            color: theme.colorScheme.primary,
            size: 28,
          ),
        ),
        const SizedBox(width: 14),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Wrap(
                crossAxisAlignment: WrapCrossAlignment.center,
                spacing: 10,
                runSpacing: 4,
                children: [
                  Text(
                    'Token 增长速率与并发监控',
                    style: theme.textTheme.headlineSmall?.copyWith(
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                  if (rates.isActive)
                    FadeTransition(
                      opacity: _pulseAnimation,
                      child: Container(
                        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
                        decoration: BoxDecoration(
                          color: const Color(0xFF34A853).withValues(alpha: 0.15),
                          borderRadius: BorderRadius.circular(16),
                          border: Border.all(
                            color: const Color(0xFF34A853).withValues(alpha: 0.45),
                          ),
                        ),
                        child: Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Container(
                              width: 8,
                              height: 8,
                              decoration: const BoxDecoration(
                                color: Color(0xFF34A853),
                                shape: BoxShape.circle,
                              ),
                            ),
                            const SizedBox(width: 6),
                            const Text(
                              '并发活跃中 · 持续消耗',
                              style: TextStyle(
                                color: Color(0xFF34A853),
                                fontSize: 12,
                                fontWeight: FontWeight.bold,
                              ),
                            ),
                          ],
                        ),
                      ),
                    )
                  else
                    Container(
                      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
                      decoration: BoxDecoration(
                        color: theme.colorScheme.outlineVariant.withValues(alpha: 0.25),
                        borderRadius: BorderRadius.circular(16),
                      ),
                      child: Text(
                        '当前空闲 · 最近活跃: ${_formatRelativeTime(rates.latestEventMs)}',
                        style: TextStyle(
                          color: theme.colorScheme.outline,
                          fontSize: 12,
                        ),
                      ),
                    ),
                ],
              ),
              const SizedBox(height: 4),
              Text(
                '实时统计多并发运行时的 Token 吞吐率、TPS、瞬时爆发峰值与费用增速',
                style: theme.textTheme.bodyMedium?.copyWith(
                  color: theme.colorScheme.outline,
                ),
              ),
            ],
          ),
        ),
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
        ),
        const SizedBox(width: 8),
        OutlinedButton.icon(
          onPressed: _isRefreshing ? null : () => _refreshRates(triggerScan: true),
          icon: const Icon(Icons.sync, size: 18),
          label: const Text('扫描日志'),
        ),
        const SizedBox(width: 8),
        IconButton.filledTonal(
          onPressed: _isRefreshing ? null : () => _refreshRates(),
          icon: _isRefreshing
              ? const SizedBox(
                  width: 18,
                  height: 18,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Icon(Icons.refresh, size: 20),
          tooltip: '刷新速率数据',
        ),
      ],
    );
  }

  Widget _buildFilterChips(BuildContext context) {
    final theme = Theme.of(context);
    return Wrap(
      spacing: 8,
      runSpacing: 4,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        Text(
          '工具过滤: ',
          style: theme.textTheme.labelMedium?.copyWith(color: theme.colorScheme.outline),
        ),
        ..._availableApps.map((app) {
          final isSelected = _selectedApps.contains(app);
          return FilterChip(
            label: Text(app),
            selected: isSelected,
            onSelected: (selected) {
              setState(() {
                if (selected) {
                  _selectedApps.add(app);
                } else {
                  _selectedApps.remove(app);
                }
              });
              _refreshRates();
            },
            visualDensity: VisualDensity.compact,
          );
        }),
      ],
    );
  }

  Widget _buildWindowSelector(BuildContext context) {
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: SegmentedButton<String>(
        segments: const [
          ButtonSegment(
            value: '1m',
            label: Text('近 1 分钟'),
            icon: Icon(Icons.flash_on_rounded, size: 16),
          ),
          ButtonSegment(
            value: '5m',
            label: Text('近 5 分钟'),
            icon: Icon(Icons.shutter_speed_rounded, size: 16),
          ),
          ButtonSegment(
            value: '15m',
            label: Text('近 15 分钟'),
            icon: Icon(Icons.timelapse_rounded, size: 16),
          ),
          ButtonSegment(
            value: '1h',
            label: Text('近 1 小时'),
            icon: Icon(Icons.history_rounded, size: 16),
          ),
          ButtonSegment(
            value: '24h',
            label: Text('近 24 小时'),
            icon: Icon(Icons.calendar_today_rounded, size: 16),
          ),
        ],
        selected: {_selectedWindow},
        onSelectionChanged: (val) {
          setState(() {
            _selectedWindow = val.first;
          });
        },
      ),
    );
  }

  Widget _buildKpiGrid(
    BuildContext context,
    ThemeProvider themeProvider,
    TokenRateOverview rates,
    RateMetric metric,
  ) {
    return LayoutBuilder(builder: (context, constraints) {
      final cols = constraints.maxWidth > 950
          ? 4
          : (constraints.maxWidth > 550 ? 2 : 1);
      return GridView.count(
        crossAxisCount: cols,
        crossAxisSpacing: 14,
        mainAxisSpacing: 14,
        shrinkWrap: true,
        physics: const NeverScrollableScrollPhysics(),
        childAspectRatio: constraints.maxWidth > 950 ? 1.55 : 1.7,
        children: [
          // 1. Overall Velocity
          _buildKpiCard(
            context,
            title: 'Token 增长速率',
            value: '${themeProvider.formatTokens(metric.tokensPerMin.round())} /min',
            subtitle:
                '${_formatTps(metric.tokensPerSec)} tok/s · 增量 ${themeProvider.formatTokens(metric.totalTokens)}',
            icon: Icons.trending_up_rounded,
            color: const Color(0xFF1A73E8),
          ),
          // 2. Input / Output Split
          _buildKpiCard(
            context,
            title: '输入 / 输出 速率分布',
            value:
                '入 ${themeProvider.formatTokens(metric.inputTokensPerMin.round())} · 出 ${themeProvider.formatTokens(metric.outputTokensPerMin.round())} /min',
            subtitle:
                '缓存命中: ${themeProvider.formatTokens(metric.cacheReadTokens)} · 写入: ${themeProvider.formatTokens(metric.cacheWriteTokens)}',
            icon: Icons.sync_alt_rounded,
            color: const Color(0xFF8E24AA),
          ),
          // 3. Burn Rate (USD / hour)
          _buildKpiCard(
            context,
            title: '预估费用增速',
            value: '\$${metric.costPerHour.toStringAsFixed(2)} /h',
            subtitle: '该窗口实际花费: \$${metric.costUsd.toStringAsFixed(4)}',
            icon: Icons.local_fire_department_rounded,
            color: const Color(0xFF34A853),
          ),
          // 4. Concurrency & Burst Peak
          _buildKpiCard(
            context,
            title: '并发请求数与峰值',
            value: '${metric.requestsPerMin.toStringAsFixed(1)} req/min',
            subtitle:
                '1h 峰值: ${themeProvider.formatTokens(rates.peak1mIn1h.tokensPerMin.round())}/min (${_formatTps(rates.peak1mIn1h.tokensPerSec)} TPS)',
            icon: Icons.bolt_rounded,
            color: const Color(0xFFFBBC05),
          ),
        ],
      );
    });
  }

  Widget _buildKpiCard(
    BuildContext context, {
    required String title,
    required String value,
    required String subtitle,
    required IconData icon,
    required Color color,
  }) {
    final theme = Theme.of(context);
    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(14),
        side: BorderSide(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.35),
        ),
      ),
      color: theme.colorScheme.surface,
      child: Padding(
        padding: const EdgeInsets.all(16),
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
                  padding: const EdgeInsets.all(6),
                  decoration: BoxDecoration(
                    color: color.withValues(alpha: 0.12),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: Icon(icon, size: 18, color: color),
                ),
              ],
            ),
            const SizedBox(height: 6),
            Text(
              value,
              style: theme.textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.bold,
                color: color,
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
                fontSize: 12,
              ),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildTimelineChart(
    BuildContext context,
    ThemeProvider themeProvider,
    TokenRateOverview rates,
  ) {
    final theme = Theme.of(context);
    final timeline = rates.timeline1h;
    final hasData = timeline.any((b) => b.totalTokens > 0);
    final maxTok = timeline.isEmpty
        ? 0
        : timeline.map((b) => b.totalTokens).reduce(math.max);
    final maxY = hasData ? (maxTok * 1.25).toDouble() : 1000.0;

    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.35),
        ),
      ),
      color: theme.colorScheme.surface,
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '近 1 小时分钟级速率走势',
                      style: theme.textTheme.titleMedium?.copyWith(
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                    const SizedBox(height: 2),
                    Text(
                      '60 分钟高精连续时间序列，清晰展现并发脉冲与波峰波谷',
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: theme.colorScheme.outline,
                      ),
                    ),
                  ],
                ),
                if (hasData)
                  Container(
                    padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                    decoration: BoxDecoration(
                      color: const Color(0xFF1A73E8).withValues(alpha: 0.1),
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(
                        color: const Color(0xFF1A73E8).withValues(alpha: 0.3),
                      ),
                    ),
                    child: Text(
                      '1h 峰值: ${themeProvider.formatTokens(rates.peak1mIn1h.tokensPerMin.round())}/min (${_formatTps(rates.peak1mIn1h.tokensPerSec)} TPS)',
                      style: const TextStyle(
                        color: Color(0xFF1A73E8),
                        fontWeight: FontWeight.bold,
                        fontSize: 12,
                      ),
                    ),
                  ),
              ],
            ),
            const SizedBox(height: 20),
            SizedBox(
              height: 220,
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
                      color: theme.colorScheme.outlineVariant.withValues(alpha: 0.2),
                      strokeWidth: 1,
                      dashArray: [4, 4],
                    ),
                  ),
                  titlesData: FlTitlesData(
                    rightTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                    topTitles: const AxisTitles(sideTitles: SideTitles(showTitles: false)),
                    leftTitles: AxisTitles(
                      sideTitles: SideTitles(
                        showTitles: true,
                        reservedSize: 48,
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
                        reservedSize: 24,
                        interval: 10,
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
                            '速率: ${themeProvider.formatTokens(b.totalTokens)} tokens/min\n'
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
                        return FlSpot(e.key.toDouble(), e.value.totalTokens.toDouble());
                      }).toList(),
                      isCurved: true,
                      curveSmoothness: 0.25,
                      color: const Color(0xFF1A73E8),
                      barWidth: 2.8,
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
      ),
    );
  }

  Widget _buildTopModelsCard(
    BuildContext context,
    ThemeProvider themeProvider,
    TokenRateOverview rates,
  ) {
    final theme = Theme.of(context);
    final models = rates.topModels1h.take(8).toList();

    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.35),
        ),
      ),
      color: theme.colorScheme.surface,
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.layers_outlined, size: 20, color: theme.colorScheme.primary),
                const SizedBox(width: 8),
                Text(
                  '高频并发模型排行',
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.bold,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 16),
            if (models.isEmpty)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 30),
                child: Center(
                  child: Text(
                    '该时间段内暂无并发模型记录',
                    style: TextStyle(color: theme.colorScheme.outline),
                  ),
                ),
              )
            else
              ...models.map((m) {
                return Container(
                  margin: const EdgeInsets.only(bottom: 10),
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: theme.colorScheme.surfaceContainerHighest.withValues(alpha: 0.25),
                    borderRadius: BorderRadius.circular(10),
                    border: Border.all(
                      color: theme.colorScheme.outlineVariant.withValues(alpha: 0.2),
                    ),
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Container(
                            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                            decoration: BoxDecoration(
                              color: theme.colorScheme.primary.withValues(alpha: 0.12),
                              borderRadius: BorderRadius.circular(4),
                            ),
                            child: Text(
                              m.app,
                              style: TextStyle(
                                fontSize: 11,
                                color: theme.colorScheme.primary,
                                fontWeight: FontWeight.bold,
                              ),
                            ),
                          ),
                          const SizedBox(width: 8),
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
                          Text(
                            '${themeProvider.formatTokens(m.tokensPerMin.round())} /min',
                            style: TextStyle(
                              fontWeight: FontWeight.bold,
                              color: theme.colorScheme.primary,
                            ),
                          ),
                          const SizedBox(width: 6),
                          Text(
                            '(${_formatTps(m.tokensPerSec)} TPS)',
                            style: TextStyle(
                              fontSize: 11,
                              color: theme.colorScheme.outline,
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 8),
                      Row(
                        children: [
                          Expanded(
                            child: ClipRRect(
                              borderRadius: BorderRadius.circular(4),
                              child: LinearProgressIndicator(
                                value: (m.percentage / 100.0).clamp(0.0, 1.0),
                                minHeight: 6,
                                backgroundColor:
                                    theme.colorScheme.outlineVariant.withValues(alpha: 0.2),
                                valueColor: AlwaysStoppedAnimation<Color>(
                                  theme.colorScheme.primary,
                                ),
                              ),
                            ),
                          ),
                          const SizedBox(width: 12),
                          Text(
                            '${m.percentage.toStringAsFixed(1)}%',
                            style: const TextStyle(fontWeight: FontWeight.w600, fontSize: 12),
                          ),
                          const SizedBox(width: 12),
                          Text(
                            '\$${m.costUsd.toStringAsFixed(2)}',
                            style: const TextStyle(
                              color: Color(0xFF34A853),
                              fontWeight: FontWeight.bold,
                              fontSize: 12,
                            ),
                          ),
                        ],
                      ),
                    ],
                  ),
                );
              }),
          ],
        ),
      ),
    );
  }

  Widget _buildTopAppsCard(
    BuildContext context,
    ThemeProvider themeProvider,
    TokenRateOverview rates,
  ) {
    final theme = Theme.of(context);
    final apps = rates.topApps1h;

    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.35),
        ),
      ),
      color: theme.colorScheme.surface,
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.apps_rounded, size: 20, color: const Color(0xFF8E24AA)),
                const SizedBox(width: 8),
                Text(
                  '并发工具速率分布',
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.bold,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 16),
            if (apps.isEmpty)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 30),
                child: Center(
                  child: Text(
                    '该时间段内暂无工具流量记录',
                    style: TextStyle(color: theme.colorScheme.outline),
                  ),
                ),
              )
            else
              ...apps.map((a) {
                return Container(
                  margin: const EdgeInsets.only(bottom: 10),
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: theme.colorScheme.surfaceContainerHighest.withValues(alpha: 0.25),
                    borderRadius: BorderRadius.circular(10),
                    border: Border.all(
                      color: theme.colorScheme.outlineVariant.withValues(alpha: 0.2),
                    ),
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          Text(
                            a.app,
                            style: theme.textTheme.bodyMedium?.copyWith(
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          Text(
                            '${themeProvider.formatTokens(a.tokensPerMin.round())} /min',
                            style: const TextStyle(
                              fontWeight: FontWeight.bold,
                              color: Color(0xFF8E24AA),
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 6),
                      Row(
                        children: [
                          Expanded(
                            child: ClipRRect(
                              borderRadius: BorderRadius.circular(4),
                              child: LinearProgressIndicator(
                                value: (a.percentage / 100.0).clamp(0.0, 1.0),
                                minHeight: 6,
                                backgroundColor:
                                    theme.colorScheme.outlineVariant.withValues(alpha: 0.2),
                                valueColor: const AlwaysStoppedAnimation<Color>(
                                  Color(0xFF8E24AA),
                                ),
                              ),
                            ),
                          ),
                          const SizedBox(width: 12),
                          Text(
                            '${a.percentage.toStringAsFixed(1)}%',
                            style: const TextStyle(fontWeight: FontWeight.w600, fontSize: 12),
                          ),
                        ],
                      ),
                    ],
                  ),
                );
              }),
          ],
        ),
      ),
    );
  }

  Widget _buildBurstInsightsCard(
    BuildContext context,
    ThemeProvider themeProvider,
    TokenRateOverview rates,
  ) {
    final theme = Theme.of(context);
    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(16),
        side: BorderSide(
          color: theme.colorScheme.outlineVariant.withValues(alpha: 0.35),
        ),
      ),
      color: theme.colorScheme.surfaceContainerHighest.withValues(alpha: 0.25),
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Icon(Icons.insights_rounded, size: 20, color: Color(0xFFFBBC05)),
                const SizedBox(width: 8),
                Text(
                  '并发爆发极值洞察',
                  style: theme.textTheme.titleMedium?.copyWith(
                    fontWeight: FontWeight.bold,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 14),
            LayoutBuilder(builder: (context, constraints) {
              final isWide = constraints.maxWidth > 650;
              final card1 = Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: theme.colorScheme.surface,
                  borderRadius: BorderRadius.circular(12),
                  border: Border.all(
                    color: theme.colorScheme.outlineVariant.withValues(alpha: 0.25),
                  ),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '近 1 小时最高 1 分钟爆发',
                      style: theme.textTheme.labelMedium?.copyWith(
                        color: theme.colorScheme.outline,
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      '${themeProvider.formatTokens(rates.peak1mIn1h.tokensPerMin.round())} /min',
                      style: theme.textTheme.titleLarge?.copyWith(
                        fontWeight: FontWeight.bold,
                        color: const Color(0xFF1A73E8),
                      ),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      '等价 ${_formatTps(rates.peak1mIn1h.tokensPerSec)} TPS · ${rates.peak1mIn1h.events} 次请求',
                      style: TextStyle(
                        fontSize: 12,
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              );

              final card2 = Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: theme.colorScheme.surface,
                  borderRadius: BorderRadius.circular(12),
                  border: Border.all(
                    color: theme.colorScheme.outlineVariant.withValues(alpha: 0.25),
                  ),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '近 24 小时最高 1 分钟爆发',
                      style: theme.textTheme.labelMedium?.copyWith(
                        color: theme.colorScheme.outline,
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      '${themeProvider.formatTokens(rates.peak1mIn24h.tokensPerMin.round())} /min',
                      style: theme.textTheme.titleLarge?.copyWith(
                        fontWeight: FontWeight.bold,
                        color: const Color(0xFFFBBC05),
                      ),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      '等价 ${_formatTps(rates.peak1mIn24h.tokensPerSec)} TPS · ${rates.peak1mIn24h.events} 次请求',
                      style: TextStyle(
                        fontSize: 12,
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  ],
                ),
              );

              if (isWide) {
                return Row(
                  children: [
                    Expanded(child: card1),
                    const SizedBox(width: 14),
                    Expanded(child: card2),
                  ],
                );
              } else {
                return Column(
                  children: [
                    card1,
                    const SizedBox(height: 12),
                    card2,
                  ],
                );
              }
            }),
          ],
        ),
      ),
    );
  }
}
