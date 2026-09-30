import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';
import '../core/theme.dart';
import '../widgets/charts/multi_model_trend_chart.dart';
import '../widgets/charts/pie_donut_chart.dart';
import '../widgets/charts/token_activity_calendar.dart';
import '../widgets/charts/trend_chart.dart';
import '../widgets/quota_card.dart';
import '../widgets/share_card.dart';
import '../widgets/stat_card.dart';

class OverviewPage extends StatefulWidget {
  const OverviewPage({super.key});

  @override
  State<OverviewPage> createState() => _OverviewPageState();
}

class _OverviewPageState extends State<OverviewPage> {
  // In-memory cache for instant 0ms switching across time ranges
  static final Map<String, OverviewData> _overviewCache = {};

  String _selectedRange = 'week';
  final List<String> _selectedApps = [];
  final List<String> _selectedModels = [];

  bool _loading = false;
  String? _error;
  OverviewData? _data;

  String _cacheKey(String range) =>
      '${range}_${_selectedApps.join(",")}_${_selectedModels.join(",")}';

  @override
  void initState() {
    super.initState();
    final key = _cacheKey(_selectedRange);
    if (_overviewCache.containsKey(key)) {
      _data = _overviewCache[key];
    }
    _loadData(_selectedRange).then((_) => _autoScanOnce());
  }

  Future<void> _loadData(String range, {bool force = false}) async {
    final key = _cacheKey(range);

    // If already cached and not forced, display immediately and silently refresh
    if (!force && _overviewCache.containsKey(key)) {
      if (mounted) {
        setState(() {
          _data = _overviewCache[key];
          _loading = false;
        });
      }
    } else {
      if (mounted) {
        setState(() {
          _loading = true;
          _error = null;
        });
      }
    }

    try {
      // FfiBridge executes SQLite query & JSON deserialization inside Isolate.run (background OS thread)
      final data = await FfiBridge.instance.getOverview(
        rangeKey: range,
        filterApps: _selectedApps.isNotEmpty ? _selectedApps : null,
        filterModels: _selectedModels.isNotEmpty ? _selectedModels : null,
      );

      _overviewCache[key] = data;

      if (mounted && _selectedRange == range) {
        setState(() {
          _data = data;
          _loading = false;
        });
      }
    } catch (e) {
      if (mounted && _selectedRange == range) {
        setState(() {
          _error = e.toString();
          _loading = false;
        });
      }
    }
  }

  Future<void> _triggerScan() async {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('正在执行增量扫描...'), duration: Duration(seconds: 1)),
    );
    try {
      final res = await FfiBridge.instance.scan();
      final ingested = res['events_ingested'] ?? 0;
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('扫描完成，新入库 $ingested 条记录')),
        );
        _overviewCache.clear();
        _loadData(_selectedRange, force: true);
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('扫描失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  /// At most once per app session: a silent background scan right after the
  /// first overview load, so usage from sources that were never scanned before
  /// (or that produced data while the app was closed) shows up without the
  /// user having to know about the manual refresh button. The scan itself
  /// runs off the UI isolate; ranges switched meanwhile simply queue on the
  /// engine behind it.
  static bool _autoScanRan = false;

  Future<void> _autoScanOnce() async {
    if (_autoScanRan) return;
    _autoScanRan = true;
    try {
      final res = await FfiBridge.instance.scan();
      final ingested = (res['events_ingested'] as num?)?.toInt() ?? 0;
      if (ingested > 0) {
        _overviewCache.clear();
        if (mounted) await _loadData(_selectedRange, force: true);
      }
    } catch (_) {
      // Silent: scan failures surface through the manual refresh button.
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final themeProvider = Provider.of<ThemeProvider>(context);

    if (_loading && _data == null) {
      return const Center(child: CircularProgressIndicator());
    }

    if (_error != null && _data == null) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.error_outline, size: 48, color: theme.colorScheme.error),
            const SizedBox(height: 16),
            Text('加载概览失败', style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
            const SizedBox(height: 16),
            FilledButton.icon(
              onPressed: () => _loadData(_selectedRange, force: true),
              icon: const Icon(Icons.refresh),
              label: const Text('重试'),
            ),
          ],
        ),
      );
    }

    final data = _data!;
    final span = data.span;

    return RefreshIndicator(
      onRefresh: () => _loadData(_selectedRange, force: true),
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(24.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Top Bar: Time Range Selector & Action Buttons
            SizedBox(
              width: double.infinity,
              child: Wrap(
                alignment: WrapAlignment.spaceBetween,
                crossAxisAlignment: WrapCrossAlignment.center,
                spacing: 12,
                runSpacing: 12,
                children: [
                  SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: SegmentedButton<String>(
                      segments: const [
                        ButtonSegment(value: 'today', label: Text('今日')),
                        ButtonSegment(value: 'week', label: Text('近 7 天')),
                        ButtonSegment(value: 'month', label: Text('近 30 天')),
                        ButtonSegment(value: 'all', label: Text('全部')),
                      ],
                      selected: {_selectedRange},
                      onSelectionChanged: (val) {
                        final newRange = val.first;
                        final key = _cacheKey(newRange);
                        // Instant optimistic update (0ms latency):
                        setState(() {
                          _selectedRange = newRange;
                          if (_overviewCache.containsKey(key)) {
                            _data = _overviewCache[key];
                            _loading = false;
                          } else {
                            _loading = true; // Show subtle progress bar, keep page mounted
                          }
                        });
                        _loadData(newRange);
                      },
                    ),
                  ),
                  Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      FilledButton.tonalIcon(
                        onPressed: _loading ? null : () => showShareCardDialog(
                          context,
                          data,
                          _selectedRange,
                        ),
                        icon: const Icon(Icons.share_outlined, size: 18),
                        label: const Text('分享卡片'),
                      ),
                      const SizedBox(width: 8),
                      OutlinedButton.icon(
                        onPressed: _triggerScan,
                        icon: const Icon(Icons.sync, size: 18),
                        label: const Text('扫描新日志'),
                      ),
                      const SizedBox(width: 8),
                      IconButton.filledTonal(
                        onPressed: () => _loadData(_selectedRange, force: true),
                        icon: const Icon(Icons.refresh, size: 20),
                        tooltip: '刷新数据',
                      ),
                    ],
                  ),
                ],
              ),
            ),
            const SizedBox(height: 6),

            // Subtle non-intrusive loading indicator
            if (_loading)
              ClipRRect(
                borderRadius: BorderRadius.circular(2),
                child: const LinearProgressIndicator(minHeight: 3),
              )
            else
              const SizedBox(height: 3),
            const SizedBox(height: 10),

            // Tool Filter Chips
            if (data.apps.isNotEmpty) ...[
              Wrap(
                spacing: 8,
                runSpacing: 4,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text("工具筛选: ",
                      style: theme.textTheme.labelMedium
                          ?.copyWith(color: theme.colorScheme.outline)),
                  ...data.apps.map((app) {
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
                        _overviewCache.clear();
                        _loadData(_selectedRange);
                      },
                      visualDensity: VisualDensity.compact,
                    );
                  }),
                ],
              ),
              const SizedBox(height: 16),
            ],

            // 4 Stat Cards Grid
            LayoutBuilder(builder: (context, constraints) {
              final crossAxisCount = constraints.maxWidth > 900
                  ? 4
                  : (constraints.maxWidth > 500 ? 2 : 1);
              return GridView.count(
                crossAxisCount: crossAxisCount,
                crossAxisSpacing: 16,
                mainAxisSpacing: 16,
                shrinkWrap: true,
                physics: const NeverScrollableScrollPhysics(),
                childAspectRatio: 1.55,
                children: [
                  StatCard(
                    title: '累计 Token 用量',
                    value: themeProvider.formatTokens(span.totalTokens),
                    subtitle:
                        '输入: ${themeProvider.formatTokens(span.inputTokens)} · 输出: ${themeProvider.formatTokens(span.outputTokens)}',
                    icon: Icons.token,
                    color: const Color(0xFF1A73E8),
                  ),
                  StatCard(
                    title: '预估费用 (USD)',
                    value: '\$${span.costUsd.toStringAsFixed(2)}',
                    subtitle: span.credits > 0
                        ? '订阅抵扣: ${span.credits.toStringAsFixed(1)} credits'
                        : '按价目表计算',
                    icon: Icons.attach_money,
                    color: const Color(0xFF34A853),
                  ),
                  StatCard(
                    title: '请求事件次数',
                    value: themeProvider.formatTokens(span.events),
                    subtitle:
                        '活跃时长: ${(span.activeMs / 1000 / 60).toStringAsFixed(1)} 分钟',
                    icon: Icons.bolt,
                    color: const Color(0xFFFBBC05),
                  ),
                  StatCard(
                    title: '缓存命中 Token',
                    value: themeProvider.formatTokens(span.cacheReadTokens),
                    subtitle: '缓存写入: ${themeProvider.formatTokens(span.cacheWriteTokens)}',
                    icon: Icons.speed,
                    color: const Color(0xFF8E24AA),
                  ),
                ],
              );
            }),
            const SizedBox(height: 28),

            // Token 活动日历热力图 (GitHub-style Contribution Heatmap)
            TokenActivityCalendar(
              activity: data.activity,
            ),
            const SizedBox(height: 24),

            // 每日 Token 多模型趋势图 (Curved Multi-Series Spline Chart)
            MultiModelTrendChart(
              daily: data.daily,
              daily30d: data.daily30d,
            ),
            const SizedBox(height: 28),

            // 饼状图/环形图 展示区 (4 大核心维度)
            Text(
              "用量与成本占比分布",
              style: theme.textTheme.titleLarge
                  ?.copyWith(fontWeight: FontWeight.bold),
            ),
            const SizedBox(height: 16),

            LayoutBuilder(builder: (context, constraints) {
              final chartCols = constraints.maxWidth > 800 ? 2 : 1;
              return GridView.count(
                 crossAxisCount: chartCols,
                crossAxisSpacing: 16,
                mainAxisSpacing: 16,
                shrinkWrap: true,
                physics: const NeverScrollableScrollPhysics(),
                childAspectRatio: constraints.maxWidth > 800 ? 1.18 : 1.05,
                children: [
                  // 1. 按工具 · 成本
                  PieDonutChart(
                    title: '按工具 · 成本分布',
                    centerUnit: '美元',
                    initialIsDonut: false,
                    valueFormatter: (v) => '\$${v.toStringAsFixed(2)}',
                    items: data.byApp
                        .map((a) => PieDonutChartItem(
                              label: a.app,
                              value: a.costUsd,
                            ))
                        .toList(),
                  ),
                  // 2. 按模型 · 成本
                  PieDonutChart(
                    title: '按模型 · 成本分布',
                    centerUnit: '美元',
                    initialIsDonut: true,
                    valueFormatter: (v) => '\$${v.toStringAsFixed(2)}',
                    items: data.byModel
                        .map((m) => PieDonutChartItem(
                              label: m.name,
                              value: m.costUsd,
                            ))
                        .toList(),
                  ),
                  // 3. 按工具 · Tokens
                  PieDonutChart(
                    title: '按工具 · Token 用量',
                    centerUnit: 'Tokens',
                    initialIsDonut: true,
                    valueFormatter: (v) => themeProvider.formatTokens(v),
                    items: data.byApp
                        .map((a) => PieDonutChartItem(
                              label: a.app,
                              value: a.totalTokens.toDouble(),
                            ))
                        .toList(),
                  ),
                  // 4. 按模型 · Tokens
                  PieDonutChart(
                    title: '按模型 · Token 用量',
                    centerUnit: 'Tokens',
                    initialIsDonut: false,
                    valueFormatter: (v) => themeProvider.formatTokens(v),
                    items: data.byModel
                        .map((m) => PieDonutChartItem(
                              label: m.name,
                              value: m.tokens.toDouble(),
                            ))
                        .toList(),
                  ),
                ],
              );
            }),
            const SizedBox(height: 28),

            // 今日小时用量时间分布 (当选择“今日”时展示细粒度柱状图)
            if (_selectedRange == 'today') ...[
              TrendChart(
                daily: data.daily,
                title: '今日小时用量分布',
              ),
              const SizedBox(height: 28),
            ],

            // 配额卡片区
            if (data.quotaGroups.isNotEmpty) ...[
              Text(
                "订阅与配额状态",
                style: theme.textTheme.titleLarge
                    ?.copyWith(fontWeight: FontWeight.bold),
              ),
              const SizedBox(height: 16),
              LayoutBuilder(builder: (context, constraints) {
                final quotaCols = constraints.maxWidth > 900
                    ? 3
                    : (constraints.maxWidth > 550 ? 2 : 1);
                return GridView.count(
                  crossAxisCount: quotaCols,
                  crossAxisSpacing: 16,
                  mainAxisSpacing: 16,
                  shrinkWrap: true,
                  physics: const NeverScrollableScrollPhysics(),
                  childAspectRatio: 1.6,
                  children: data.quotaGroups
                      .map((g) => QuotaCardWidget(group: g))
                      .toList(),
                );
              }),
            ],
          ],
        ),
      ),
    );
  }
}
