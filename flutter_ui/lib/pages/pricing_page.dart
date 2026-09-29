import 'package:flutter/material.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';

class PricingPage extends StatefulWidget {
  const PricingPage({Key? key}) : super(key: key);

  @override
  State<PricingPage> createState() => _PricingPageState();
}

class _PricingPageState extends State<PricingPage> {
  bool _loading = true;
  String? _error;
  List<PriceRow> _prices = [];
  String _searchQuery = '';

  @override
  void initState() {
    super.initState();
    _loadPrices();
  }

  Future<void> _loadPrices() async {
    setState(() {
      _loading = true;
      _error = null;
    });

    try {
      final list = await FfiBridge.instance.getPrices();
      if (mounted) {
        setState(() {
          _prices = list;
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

  Future<void> _updatePrices() async {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('正在向 llmpricing.dev 同步最新模型价目...'), duration: Duration(seconds: 2)),
    );
    try {
      final res = await FfiBridge.instance.updatePrices();
      final repriced = res['repriced'] ?? 0;
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('价目同步完成，重新计算了 $repriced 条未定价事件')),
        );
        _loadPrices();
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
      return p.model.toLowerCase().contains(_searchQuery.toLowerCase()) ||
          p.source.toLowerCase().contains(_searchQuery.toLowerCase());
    }).toList();

    return Padding(
      padding: const EdgeInsets.all(24.0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                'LLM 模型价目表 (每 1M Tokens 美元价格)',
                style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
              ),
              FilledButton.icon(
                onPressed: _updatePrices,
                icon: const Icon(Icons.cloud_download),
                label: const Text('在线同步最新价目'),
              ),
            ],
          ),
          const SizedBox(height: 16),
          TextField(
            decoration: InputDecoration(
              hintText: '搜索模型名称或来源 (如 gpt-4o, claude-3-7, deepseek)...',
              prefixIcon: const Icon(Icons.search),
              border: OutlineInputBorder(borderRadius: BorderRadius.circular(12)),
              filled: true,
              fillColor: theme.colorScheme.surface,
              contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
            ),
            onChanged: (val) => setState(() => _searchQuery = val),
          ),
          const SizedBox(height: 16),
          Expanded(
            child: Card(
              elevation: 1,
              child: _loading && _prices.isEmpty
                  ? const Center(child: CircularProgressIndicator())
                  : filtered.isEmpty
                      ? Center(child: Text("未找到匹配的模型价格", style: TextStyle(color: theme.colorScheme.outline)))
                      : SingleChildScrollView(
                          child: DataTable(
                            headingRowColor: MaterialStateProperty.all(theme.colorScheme.surfaceVariant.withOpacity(0.5)),
                            columns: const [
                              DataColumn(label: Text('模型名称')),
                              DataColumn(label: Text('输入 (\$ / 1M)')),
                              DataColumn(label: Text('输出 (\$ / 1M)')),
                              DataColumn(label: Text('缓存读取 (\$ / 1M)')),
                              DataColumn(label: Text('来源')),
                            ],
                            rows: filtered.map((p) {
                              return DataRow(cells: [
                                DataCell(Text(p.model, style: const TextStyle(fontWeight: FontWeight.bold))),
                                DataCell(Text('\$${p.input.toStringAsFixed(2)}')),
                                DataCell(Text('\$${p.output.toStringAsFixed(2)}')),
                                DataCell(Text('\$${p.cacheRead.toStringAsFixed(2)}')),
                                DataCell(Chip(label: Text(p.source, style: const TextStyle(fontSize: 11)), visualDensity: VisualDensity.compact)),
                              ]);
                            }).toList(),
                          ),
                        ),
            ),
          ),
        ],
      ),
    );
  }
}
