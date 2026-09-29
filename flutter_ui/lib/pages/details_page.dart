import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';

class DetailsPage extends StatefulWidget {
  const DetailsPage({Key? key}) : super(key: key);

  @override
  State<DetailsPage> createState() => _DetailsPageState();
}

class _DetailsPageState extends State<DetailsPage> {
  int _currentPage = 0;
  final int _pageSize = 50;
  bool _loading = true;
  String? _error;
  DetailData? _data;

  @override
  void initState() {
    super.initState();
    _loadPage(0);
  }

  Future<void> _loadPage(int page) async {
    setState(() {
      _loading = true;
      _error = null;
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
          _error = e.toString();
          _loading = false;
        });
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
    final fmtTokens = NumberFormat('#,###');

    if (_loading && _data == null) {
      return const Center(child: CircularProgressIndicator());
    }

    final totalEvents = _data?.totalEvents ?? 0;
    final totalPages = (totalEvents / _pageSize).ceil();
    final rows = _data?.rows ?? [];

    return Padding(
      padding: const EdgeInsets.all(24.0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                '事件明细 (共 $totalEvents 条)',
                style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
              ),
              Row(
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
          const SizedBox(height: 16),
          Expanded(
            child: Card(
              elevation: 1,
              child: rows.isEmpty
                  ? Center(child: Text("暂无事件明细", style: TextStyle(color: theme.colorScheme.outline)))
                  : SingleChildScrollView(
                      scrollDirection: Axis.horizontal,
                      child: SingleChildScrollView(
                        child: DataTable(
                          headingRowColor: MaterialStateProperty.all(theme.colorScheme.surfaceVariant.withOpacity(0.5)),
                          columns: const [
                            DataColumn(label: Text('时间')),
                            DataColumn(label: Text('工具')),
                            DataColumn(label: Text('模型')),
                            DataColumn(label: Text('总 Tokens')),
                            DataColumn(label: Text('输入 / 缓存读')),
                            DataColumn(label: Text('输出')),
                            DataColumn(label: Text('预估成本 (USD)')),
                            DataColumn(label: Text('耗时')),
                          ],
                          rows: rows.map((row) {
                            return DataRow(cells: [
                              DataCell(Text(_formatTs(row.tsStart), style: const TextStyle(fontSize: 12))),
                              DataCell(Chip(
                                label: Text(row.app, style: const TextStyle(fontSize: 11)),
                                visualDensity: VisualDensity.compact,
                              )),
                              DataCell(Text(row.model ?? '-', style: const TextStyle(fontWeight: FontWeight.w500))),
                              DataCell(Text(fmtTokens.format(row.totalTokens), style: const TextStyle(fontWeight: FontWeight.bold))),
                              DataCell(Text('${fmtTokens.format(row.inputTokens)} / ${fmtTokens.format(row.cacheReadTokens)}', style: TextStyle(color: theme.colorScheme.outline))),
                              DataCell(Text(fmtTokens.format(row.outputTokens))),
                              DataCell(Text(row.costUsd != null ? '\$${row.costUsd!.toStringAsFixed(4)}' : '-', style: TextStyle(color: Colors.green.shade700, fontWeight: FontWeight.bold))),
                              DataCell(Text(row.durationMs != null ? '${row.durationMs}ms' : '-')),
                            ]);
                          }).toList(),
                        ),
                      ),
                    ),
            ),
          ),
        ],
      ),
    );
  }
}
