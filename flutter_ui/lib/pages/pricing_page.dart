import 'dart:async';

import 'package:flutter/material.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';

class PricingPage extends StatefulWidget {
  const PricingPage({super.key});

  @override
  State<PricingPage> createState() => _PricingPageState();
}

class _PricingPageState extends State<PricingPage> {
  static List<PriceRow>? _cachedPrices;
  static int? _cachedPriceRevision;
  bool _loading = false;
  List<PriceRow> _prices = [];
  String _searchQuery = '';
  late final StreamSubscription<int> _priceUpdatesSubscription;

  @override
  void initState() {
    super.initState();
    final bridge = FfiBridge.instance;
    _priceUpdatesSubscription = bridge.priceUpdates.listen((_) {
      unawaited(_loadPrices(force: true));
    });
    if (_cachedPrices != null && _cachedPriceRevision == bridge.priceRevision) {
      _prices = _cachedPrices!;
    } else {
      unawaited(_loadPrices(force: true));
    }
  }

  @override
  void dispose() {
    unawaited(_priceUpdatesSubscription.cancel());
    super.dispose();
  }

  Future<void> _loadPrices({bool force = false}) async {
    if (!mounted) return;

    if (!force && _cachedPrices != null) {
      setState(() {
        _prices = _cachedPrices!;
        _loading = false;
      });
      return;
    }

    setState(() {
      _loading = true;
    });

    try {
      final bridge = FfiBridge.instance;
      final revision = bridge.priceRevision;
      final list = await bridge.getPrices();
      if (revision != bridge.priceRevision) {
        if (!mounted) return;
        await _loadPrices(force: true);
        return;
      }
      _cachedPrices = list;
      _cachedPriceRevision = revision;
      if (mounted) {
        setState(() {
          _prices = list;
          _loading = false;
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _loading = false;
        });
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('加载价格表失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  Future<void> _updatePrices() async {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('正在同步在线模型价目...'), duration: Duration(seconds: 2)),
    );
    try {
      final res = await FfiBridge.instance.updatePrices();
      final repriced = res['repriced'] ?? 0;
      final failed = (res['failed'] as List?)?.length ?? 0;
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text(
              failed == 0
                  ? '价目同步完成，重新计算了 $repriced 条事件'
                  : '价目已同步，$failed 个数据源失败，重新计算了 $repriced 条事件',
            ),
          ),
        );
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('价目同步失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    final filtered = _prices.where((p) {
      if (_searchQuery.isEmpty) return true;
      final q = _searchQuery.toLowerCase();
      return p.model.toLowerCase().contains(q) || p.source.toLowerCase().contains(q);
    }).toList();

    final headerStyle = theme.textTheme.bodyMedium?.copyWith(
      fontWeight: FontWeight.bold,
      color: theme.colorScheme.onSurfaceVariant,
    );

    return Padding(
      padding: const EdgeInsets.all(24.0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // Header Bar
          SizedBox(
            width: double.infinity,
            child: Wrap(
              alignment: WrapAlignment.spaceBetween,
              crossAxisAlignment: WrapCrossAlignment.center,
              spacing: 12,
              runSpacing: 12,
              children: [
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'LLM 模型价目表 (${_prices.length} 个模型)',
                      style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      '单位：美元 / 1M Tokens (支持模糊实时检索)',
                      style: TextStyle(color: theme.colorScheme.outline, fontSize: 12),
                    ),
                  ],
                ),
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton.filledTonal(
                      onPressed: () => _loadPrices(force: true),
                      icon: const Icon(Icons.refresh, size: 20),
                      tooltip: '重新加载本地价格',
                    ),
                    const SizedBox(width: 8),
                    FilledButton.icon(
                      onPressed: _updatePrices,
                      icon: const Icon(Icons.cloud_download, size: 18),
                      label: const Text('在线同步最新价目'),
                    ),
                  ],
                ),
              ],
            ),
          ),
          const SizedBox(height: 16),

          // Search Box
          TextField(
            decoration: InputDecoration(
              hintText: '输入模型名称搜索 (例如: claude-3-7, gpt-4o, deepseek-v3, gemini)...',
              prefixIcon: const Icon(Icons.search),
              border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
              filled: true,
              fillColor: theme.colorScheme.surface,
              contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
              suffixIcon: _searchQuery.isNotEmpty
                  ? IconButton(
                      icon: const Icon(Icons.clear, size: 18),
                      onPressed: () => setState(() => _searchQuery = ''),
                    )
                  : null,
            ),
            onChanged: (val) => setState(() => _searchQuery = val),
          ),
          const SizedBox(height: 16),

          // Virtualized High-Performance Table Container
          Expanded(
            child: Card(
              elevation: 1,
              clipBehavior: Clip.antiAlias,
              child: _loading && _prices.isEmpty
                  ? const Center(child: CircularProgressIndicator())
                  : Column(
                      children: [
                        // Pinned Table Header
                        Container(
                          height: 44,
                          padding: const EdgeInsets.symmetric(horizontal: 20),
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
                              Expanded(flex: 4, child: Text('模型标识 (Model ID)', style: headerStyle)),
                              Expanded(flex: 2, child: Text('输入 (\$ / 1M)', style: headerStyle)),
                              Expanded(flex: 2, child: Text('输出 (\$ / 1M)', style: headerStyle)),
                              Expanded(flex: 2, child: Text('缓存读取 (\$ / 1M)', style: headerStyle)),
                              Expanded(flex: 2, child: Text('数据来源', style: headerStyle)),
                            ],
                          ),
                        ),

                        // Virtualized Table Body (ListView.builder with itemExtent: 42.0)
                        // Extremely light memory & CPU footprint: only ~15 rows instantiated at once!
                        Expanded(
                          child: filtered.isEmpty
                              ? Center(
                                  child: Text(
                                    _searchQuery.isNotEmpty ? '未找到匹配的模型' : '暂无价目数据',
                                    style: TextStyle(color: theme.colorScheme.outline),
                                  ),
                                )
                              : ListView.builder(
                                  itemCount: filtered.length,
                                  itemExtent: 42.0,
                                  itemBuilder: (context, i) {
                                    final p = filtered[i];
                                    final isEven = i % 2 == 0;

                                    return Container(
                                      padding: const EdgeInsets.symmetric(horizontal: 20),
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
                                            flex: 4,
                                            child: Text(
                                              p.model,
                                              style: const TextStyle(fontWeight: FontWeight.w600, fontSize: 13),
                                              maxLines: 1,
                                              overflow: TextOverflow.ellipsis,
                                            ),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Text(
                                              '\$${p.input.toStringAsFixed(2)}',
                                              style: const TextStyle(fontSize: 13),
                                            ),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Text(
                                              '\$${p.output.toStringAsFixed(2)}',
                                              style: const TextStyle(fontSize: 13),
                                            ),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Text(
                                              '\$${p.cacheRead.toStringAsFixed(2)}',
                                              style: TextStyle(
                                                fontSize: 13,
                                                color: p.cacheRead > 0 ? Colors.green.shade700 : theme.colorScheme.outline,
                                              ),
                                            ),
                                          ),
                                          Expanded(
                                            flex: 2,
                                            child: Align(
                                              alignment: Alignment.centerLeft,
                                              child: Container(
                                                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                                                decoration: BoxDecoration(
                                                  color: theme.colorScheme.secondaryContainer.withValues(alpha: 0.6),
                                                  borderRadius: BorderRadius.circular(6),
                                                ),
                                                child: Text(
                                                  p.source,
                                                  style: TextStyle(
                                                    fontSize: 11,
                                                    fontWeight: FontWeight.w500,
                                                    color: theme.colorScheme.onSecondaryContainer,
                                                  ),
                                                ),
                                              ),
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
