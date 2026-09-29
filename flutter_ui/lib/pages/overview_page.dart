import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';
import '../widgets/charts/pie_donut_chart.dart';
import '../widgets/charts/trend_chart.dart';
import '../widgets/quota_card.dart';
import '../widgets/stat_card.dart';

class OverviewPage extends StatefulWidget {
  const OverviewPage({Key? key}) : super(key: key);

  @override
  State<OverviewPage> createState() => _OverviewPageState();
}

class _OverviewPageState extends State<OverviewPage> {
  String _selectedRange = 'week';
  final List<String> _selectedApps = [];
  final List<String> _selectedModels = [];

  bool _loading = true;
  String? _error;
  OverviewData? _data;

  @override
  void initState() {
    super.initState();
    _loadData();
  }

  Future<void> _loadData() async {
    setState(() {
      _loading = true;
      _error = null;
    });

    try {
      final data = await FfiBridge.instance.getOverview(
        rangeKey: _selectedRange,
        filterApps: _selectedApps.isNotEmpty ? _selectedApps : null,
        filterModels: _selectedModels.isNotEmpty ? _selectedModels : null,
      );
      if (mounted) {
        setState(() {
          _data = data;
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
        _loadData();
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('扫描失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

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
              onPressed: _loadData,
              icon: const Icon(Icons.refresh),
              label: const Text('重试'),
            ),
          ],
        ),
      );
    }

    final data = _data!;
    final span = data.span;
    final fmtTokens = NumberFormat('#,###');

    return RefreshIndicator(
      onRefresh: _loadData,
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(24.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Top Bar: Time Range Selector & Action Buttons
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                SegmentedButton<String>(
                  segments: const [
                    ButtonSegment(value: 'today', label: Text('今日')),
                    ButtonSegment(value: 'week', label: Text('近 7 天')),
                    ButtonSegment(value: 'month', label: Text('近 30 天')),
                    ButtonSegment(value: 'all', label: Text('全部')),
                  ],
                  selected: {_selectedRange},
                  onSelectionChanged: (val) {
                    setState(() {
                      _selectedRange = val.first;
                    });
                    _loadData();
                  },
                ),
                Row(
                  children: [
                    OutlinedButton.icon(
                      onPressed: _triggerScan,
                      icon: const Icon(Icons.sync, size: 18),
                      label: const Text('扫描新日志'),
                    ),
                    const SizedBox(width: 8),
                    IconButton.filledTonal(
                      onPressed: _loadData,
                      icon: const Icon(Icons.refresh, size: 20),
                      tooltip: '刷新数据',
                    ),
                  ],
                ),
              ],
            ),
            const SizedBox(height: 16),

            // Tool Filter Chips
            if (data.apps.isNotEmpty) ...[
              Wrap(
                spacing: 8,
                runSpacing: 4,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text("工具筛选: ", style: theme.textTheme.labelMedium?.copyWith(color: theme.colorScheme.outline)),
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
                        _loadData();
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
              final crossAxisCount = constraints.maxWidth > 900 ? 4 : (constraints.maxWidth > 500 ? 2 : 1);
              return GridView.count(
                crossAxisCount: crossAxisCount,
                crossAxisSpacing: 16,
                mainAxisSpacing: 16,
                shrinkWrap: true,
                physics: const NeverScrollableScrollPhysics(),
                childAspectRatio: 1.8,
                children: [
                  StatCard(
                    title: '累计 Token 用量',
                    value: fmtTokens.format(span.totalTokens),
                    subtitle: '输入: ${fmtTokens.format(span.inputTokens)} · 输出: ${fmtTokens.format(span.outputTokens)}',
                    icon: Icons.token,
                    color: const Color(0xFF1A73E8),
                  ),
                  StatCard(
                    title: '预估费用 (USD)',
                    value: '\$${span.costUsd.toStringAsFixed(2)}',
                    subtitle: span.credits > 0 ? '订阅抵扣: ${span.credits.toStringAsFixed(1)} credits' : '按价目表计算',
                    icon: Icons.attach_money,
                    color: const Color(0xFF34A853),
                  ),
                  StatCard(
                    title: '请求事件次数',
                    value: fmtTokens.format(span.events),
                    subtitle: '活跃时长: ${(span.activeMs / 1000 / 60).toStringAsFixed(1)} 分钟',
                    icon: Icons.bolt,
                    color: const Color(0xFFFBBC05),
                  ),
                  StatCard(
                    title: '缓存命中 Token',
                    value: fmtTokens.format(span.cacheReadTokens),
                    subtitle: '缓存写入: ${fmtTokens.format(span.cacheWriteTokens)}',
                    icon: Icons.speed,
                    color: const Color(0xFF8E24AA),
                  ),
                ],
              );
            }),
            const SizedBox(height: 28),

            // 饼状图/环形图 展示区 (4 大核心维度)
            Text(
              "用量与成本占比分布",
              style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
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
                childAspectRatio: constraints.maxWidth > 800 ? 1.35 : 1.1,
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
                    valueFormatter: (v) => fmtTokens.format(v.toInt()),
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
                    valueFormatter: (v) => fmtTokens.format(v.toInt()),
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

            // 30 日趋势图
            TrendChart(
              daily: data.daily,
              title: _selectedRange == 'today' ? '今日用量时间分布' : '每日 Token 趋势',
            ),
            const SizedBox(height: 28),

            // 配额卡片区
            if (data.quotaGroups.isNotEmpty) ...[
              Text(
                "订阅与配额状态",
                style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
              ),
              const SizedBox(height: 16),
              LayoutBuilder(builder: (context, constraints) {
                final quotaCols = constraints.maxWidth > 900 ? 3 : (constraints.maxWidth > 550 ? 2 : 1);
                return GridView.count(
                  crossAxisCount: quotaCols,
                  crossAxisSpacing: 16,
                  mainAxisSpacing: 16,
                  shrinkWrap: true,
                  physics: const NeverScrollableScrollPhysics(),
                  childAspectRatio: 1.6,
                  children: data.quotaGroups.map((g) => QuotaCardWidget(group: g)).toList(),
                );
              }),
            ],
          ],
        ),
      ),
    );
  }
}
