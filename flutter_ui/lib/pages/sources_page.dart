import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';

class SourcesPage extends StatefulWidget {
  const SourcesPage({Key? key}) : super(key: key);

  @override
  State<SourcesPage> createState() => _SourcesPageState();
}

class _SourcesPageState extends State<SourcesPage> {
  bool _loading = true;
  String? _error;
  List<SourceHealth> _sources = [];

  @override
  void initState() {
    super.initState();
    _loadSources();
  }

  Future<void> _loadSources() async {
    setState(() {
      _loading = true;
      _error = null;
    });

    try {
      final list = await FfiBridge.instance.getSources();
      if (mounted) {
        setState(() {
          _sources = list;
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
    if (ts == null) return '从未同步';
    final dt = DateTime.fromMillisecondsSinceEpoch(ts);
    return DateFormat('yyyy-MM-dd HH:mm:ss').format(dt);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    if (_loading && _sources.isEmpty) {
      return const Center(child: CircularProgressIndicator());
    }

    return Padding(
      padding: const EdgeInsets.all(24.0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                '数据源与日志扫描器健康度',
                style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
              ),
              IconButton.filledTonal(
                onPressed: _loadSources,
                icon: const Icon(Icons.refresh),
                tooltip: '刷新状态',
              ),
            ],
          ),
          const SizedBox(height: 20),
          Expanded(
            child: ListView.separated(
              itemCount: _sources.length,
              separatorBuilder: (_, __) => const SizedBox(height: 12),
              itemBuilder: (context, i) {
                final s = _sources[i];
                return Card(
                  elevation: 1,
                  child: ListTile(
                    contentPadding: const EdgeInsets.symmetric(horizontal: 20, vertical: 8),
                    leading: CircleAvatar(
                      backgroundColor: s.enabled ? Colors.green.shade100 : Colors.grey.shade200,
                      child: Icon(
                        s.enabled ? Icons.check_circle : Icons.pause_circle_outline,
                        color: s.enabled ? Colors.green.shade700 : Colors.grey,
                      ),
                    ),
                    title: Text(s.source, style: const TextStyle(fontWeight: FontWeight.bold)),
                    subtitle: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        const SizedBox(height: 4),
                        Text('发现文件: ${s.filesSeen} · 已处理游标: ${s.cursors} · 入库行数: ${s.rowsIngested}'),
                        Text('最近扫描: ${_formatTs(s.lastSyncedAt)}', style: TextStyle(color: theme.colorScheme.outline, fontSize: 12)),
                        if (s.lastError != null)
                          Text('异常: ${s.lastError}', style: TextStyle(color: theme.colorScheme.error, fontSize: 12)),
                      ],
                    ),
                    trailing: Chip(
                      label: Text(s.enabled ? '正常监视中' : '已停用'),
                      backgroundColor: s.enabled ? Colors.green.withOpacity(0.12) : null,
                    ),
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}
