import 'dart:async';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';
import '../core/theme.dart';

class DetailsPage extends StatefulWidget {
  const DetailsPage({super.key});

  @override
  State<DetailsPage> createState() => _DetailsPageState();
}

class _DetailsPageState extends State<DetailsPage> {
  int _currentPage = 0;
  final int _pageSize = 50;
  bool _loading = true;
  DetailData? _data;
  late final StreamSubscription<int> _priceUpdatesSubscription;

  @override
  void initState() {
    super.initState();
    _priceUpdatesSubscription = FfiBridge.instance.priceUpdates.listen((_) {
      if (mounted) unawaited(_loadPage(_currentPage));
    });
    _loadPage(0);
  }

  @override
  void dispose() {
    unawaited(_priceUpdatesSubscription.cancel());
    super.dispose();
  }

  Future<void> _loadPage(int page) async {
    setState(() {
      _loading = true;
    });

    try {
      final res = await FfiBridge.instance.getDetails(page: page, pageSize: _pageSize);
      if (mounted) {
        setState(() {
          _data = res;
          _currentPage = page;
          _loading = false;
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _loading = false;
        });
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('加载明细失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  String _formatTs(int? ts) {
    if (ts == null) return '-';
    final dt = DateTime.fromMillisecondsSinceEpoch(ts);
    return DateFormat('yyyy-MM-dd HH:mm:ss').format(dt);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final themeProvider = Provider.of<ThemeProvider>(context);

    final totalEvents = _data?.totalEvents ?? 0;
    final totalPages = (totalEvents / _pageSize).ceil();
    final rows = _data?.rows ?? [];

    final headerStyle = theme.textTheme.bodyMedium?.copyWith(
      fontWeight: FontWeight.bold,
      color: theme.colorScheme.onSurfaceVariant,
    );

    return Padding(
      padding: const EdgeInsets.all(24.0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: double.infinity,
            child: Wrap(
              alignment: WrapAlignment.spaceBetween,
              crossAxisAlignment: WrapCrossAlignment.center,
              spacing: 12,
              runSpacing: 12,
              children: [
                Text(
                  '事件明细 (共 $totalEvents 条)',
                  style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
                ),
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton.filledTonal(
                      onPressed: _currentPage > 0 ? () => _loadPage(_currentPage - 1) : null,
                      icon: const Icon(Icons.chevron_left),
                      tooltip: '上一页',
                    ),
                    const SizedBox(width: 8),
                    Text('第 ${_currentPage + 1} / ${totalPages > 0 ? totalPages : 1} 页'),
                    const SizedBox(width: 8),
                    IconButton.filledTonal(
                      onPressed: _currentPage < totalPages - 1 ? () => _loadPage(_currentPage + 1) : null,
                      icon: const Icon(Icons.chevron_right),
                      tooltip: '下一页',
                    ),
                    const SizedBox(width: 12),
                    IconButton(
                      onPressed: () => _loadPage(_currentPage),
                      icon: const Icon(Icons.refresh),
                      tooltip: '刷新',
                    ),
                  ],
                ),
              ],
            ),
          ),
          const SizedBox(height: 16),
          Expanded(
            child: Card(
              elevation: 1,
              clipBehavior: Clip.antiAlias,
              child: _loading && rows.isEmpty
                  ? const Center(child: CircularProgressIndicator())
                  : Column(
                      children: [
                        // Pinned Table Header
                        Container(
                          height: 44,
                          padding: const EdgeInsets.symmetric(horizontal: 16),
                          decoration: BoxDecoration(
                            color: theme.colorScheme.surfaceVariant.withValues(alpha: 0.6),
                            border: Border(
                              bottom: BorderSide(
                                color: theme.colorScheme.outlineVariant.withValues(alpha: 0.5),
                              ),
                            ),
                          ),
                          child: Row(
                            children: [
                              Expanded(flex: 3, child: Text('时间', style: headerStyle)),
                              Expanded(flex: 2, child: Text('工具', style: headerStyle)),
                              Expanded(flex: 4, child: Text('模型', style: headerStyle)),
                              Expanded(flex: 2, child: Text('总 Tokens', style: headerStyle)),
                              Expanded(flex: 3, child: Text('输入 / 缓存读', style: headerStyle)),
                              Expanded(flex: 2, child: Text('输出', style: headerStyle)),
                              Expanded(flex: 2, child: Text('预估成本 (USD)', style: headerStyle)),
                              Expanded(flex: 2, child: Text('耗时', style: headerStyle)),
                            ],
                          ),
                        ),

                        // Virtualized Table Body
                        Expanded(
                          child: rows.isEmpty
                              ? Center(child: Text("暂无事件明细", style: TextStyle(color: theme.colorScheme.outline)))
                              : ListView.builder(
                                  itemCount: rows.length,
                                  itemExtent: 44.0,
                                  itemBuilder: (context, i) {
                                    final row = rows[i];
                                    final isEven = i % 2 == 0;

                                    return Container(
                                      padding: const EdgeInsets.symmetric(horizontal: 16),
                                      decoration: BoxDecoration(
                                        color: isEven
                                            ? Colors.transparent
                                            : theme.colorScheme.surfaceVariant.withValues(alpha: 0.15),
                                        border: Border(
                                          bottom: BorderSide(
                                            color: theme.colorScheme.outlineVariant.withValues(alpha: 0.2),
                                          ),
                                        ),
                                      ),
                                      child: Row(
                                        children: [
                                          Expanded(
                                            flex: 3,
                                            child: Text(_formatTs(row.tsStart), style: const TextStyle(fontSize: 12)),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Align(
                                              alignment: Alignment.centerLeft,
                                              child: Container(
                                                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                                                decoration: BoxDecoration(
                                                  color: theme.colorScheme.primaryContainer.withValues(alpha: 0.5),
                                                  borderRadius: BorderRadius.circular(4),
                                                ),
                                                child: Text(row.app, style: const TextStyle(fontSize: 11)),
                                              ),
                                            ),
                                          ),
                                          Expanded(
                                            flex: 4,
                                            child: Text(
                                              row.model ?? '-',
                                              style: const TextStyle(fontWeight: FontWeight.w500, fontSize: 12),
                                              maxLines: 1,
                                              overflow: TextOverflow.ellipsis,
                                            ),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Text(
                                              themeProvider.formatTokens(row.totalTokens),
                                              style: const TextStyle(fontWeight: FontWeight.bold, fontSize: 12),
                                            ),
                                          ),
                                          Expanded(
                                            flex: 3,
                                            child: Text(
                                              '${themeProvider.formatTokens(row.inputTokens)} / ${themeProvider.formatTokens(row.cacheReadTokens)}',
                                              style: TextStyle(color: theme.colorScheme.outline, fontSize: 11),
                                            ),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Text(
                                              themeProvider.formatTokens(row.outputTokens),
                                              style: const TextStyle(fontSize: 12),
                                            ),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Text(
                                              row.costUsd != null ? '\$${row.costUsd!.toStringAsFixed(4)}' : '-',
                                              style: TextStyle(
                                                color: Colors.green.shade700,
                                                fontWeight: FontWeight.bold,
                                                fontSize: 12,
                                              ),
                                            ),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Text(
                                              row.durationMs != null ? '${row.durationMs}ms' : '-',
                                              style: const TextStyle(fontSize: 12),
                                            ),
                                          ),
                                        ],
                                      ),
                                    );
                                  },
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
}
