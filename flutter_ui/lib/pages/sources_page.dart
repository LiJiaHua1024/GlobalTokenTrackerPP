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
          SizedBox(
            width: double.infinity,
            child: Wrap(
              alignment: WrapAlignment.spaceBetween,
              crossAxisAlignment: WrapCrossAlignment.center,
              spacing: 12,
              runSpacing: 12,
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
          ),
          const SizedBox(height: 20),
          Expanded(
            child: ListView.separated(
              itemCount: _sources.length,
              separatorBuilder: (_, __) => const SizedBox(height: 12),
              itemBuilder: (context, i) {
                final s = _sources[i];
                // `enabled` is a config toggle (sources.enabled), not a health
                // signal — touch_source never rewrites it after insert. Health
                // has to come from lastError, so a failing scan must not keep
                // the green "正常监视中" chip and the green check.
                final hasErr = s.lastError != null;
                late final Color avatarBg;
                late final Color avatarFg;
                late final IconData avatarIcon;
                if (hasErr) {
                  avatarBg = theme.colorScheme.errorContainer;
                  avatarFg = theme.colorScheme.onErrorContainer;
                  avatarIcon = Icons.error_outline;
                } else if (s.enabled) {
                  avatarBg = Colors.green.shade100;
                  avatarFg = Colors.green.shade700;
                  avatarIcon = Icons.check_circle;
                } else {
                  avatarBg = Colors.grey.shade200;
                  avatarFg = Colors.grey;
                  avatarIcon = Icons.pause_circle_outline;
                }
                final chipLabel = hasErr
                    ? '扫描出错'
                    : (s.enabled ? '正常监视中' : '已停用');
                final Color? chipBg = hasErr
                    ? theme.colorScheme.error.withValues(alpha: 0.12)
                    : (s.enabled ? Colors.green.withValues(alpha: 0.12) : null);
                return Card(
                  elevation: 1,
                  child: ListTile(
                    contentPadding: const EdgeInsets.symmetric(horizontal: 20, vertical: 8),
                    leading: CircleAvatar(
                      backgroundColor: avatarBg,
                      child: Icon(avatarIcon, color: avatarFg),
                    ),
                    title: Text(s.source, style: const TextStyle(fontWeight: FontWeight.bold)),
                    subtitle: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        const SizedBox(height: 4),
                        Text('发现文件: ${s.filesSeen} · 已处理游标: ${s.cursors} · 入库行数: ${s.rowsIngested}'),
                        Text('最近扫描: ${_formatTs(s.lastSyncedAt)}', style: TextStyle(color: theme.colorScheme.outline, fontSize: 12)),
                        if (hasErr)
                          Text('异常: ${s.lastError}', style: TextStyle(color: theme.colorScheme.error, fontSize: 12)),
                      ],
                    ),
                    trailing: Chip(
                      label: Text(chipLabel),
                      backgroundColor: chipBg,
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
