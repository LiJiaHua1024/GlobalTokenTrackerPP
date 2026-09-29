import 'package:flutter/material.dart';
import '../core/ffi_bridge.dart';
import '../core/models.dart';
import '../widgets/quota_card.dart';

class QuotasPage extends StatefulWidget {
  const QuotasPage({Key? key}) : super(key: key);

  @override
  State<QuotasPage> createState() => _QuotasPageState();
}

class _QuotasPageState extends State<QuotasPage> {
  bool _loading = true;
  String? _error;
  List<QuotaGroup> _groups = [];

  @override
  void initState() {
    super.initState();
    _loadQuotas();
  }

  Future<void> _loadQuotas() async {
    setState(() {
      _loading = true;
      _error = null;
    });

    try {
      final overview = await FfiBridge.instance.getOverview();
      if (mounted) {
        setState(() {
          _groups = overview.quotaGroups;
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

  Future<void> _pollQuotas() async {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('正在向各平台获取最新配额...'), duration: Duration(seconds: 1)),
    );
    try {
      final res = await FfiBridge.instance.pollQuotas();
      final updated = res['updated'] ?? 0;
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('配额更新完毕，已同步 $updated 项')),
        );
        _loadQuotas();
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('配额同步失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    if (_loading && _groups.isEmpty) {
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
                '各工具订阅与配额监控',
                style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
              ),
              FilledButton.icon(
                onPressed: _pollQuotas,
                icon: const Icon(Icons.cloud_sync),
                label: const Text('立即同步配额'),
              ),
            ],
          ),
          const SizedBox(height: 20),
          if (_groups.isEmpty)
            Expanded(
              child: Center(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(Icons.inventory_2_outlined, size: 48, color: theme.colorScheme.outline),
                    const SizedBox(height: 12),
                    Text('暂无活跃配额记录', style: theme.textTheme.titleMedium),
                    const SizedBox(height: 6),
                    Text(
                      '支持从本地读取 Codex (~/.codex/auth.json) 和 Cursor 订阅配额',
                      style: TextStyle(color: theme.colorScheme.outline),
                    ),
                  ],
                ),
              ),
            )
          else
            Expanded(
              child: GridView.count(
                crossAxisCount: MediaQuery.of(context).size.width > 900 ? 3 : (MediaQuery.of(context).size.width > 600 ? 2 : 1),
                crossAxisSpacing: 16,
                mainAxisSpacing: 16,
                childAspectRatio: 1.5,
                children: _groups.map((g) => QuotaCardWidget(group: g)).toList(),
              ),
            ),
        ],
      ),
    );
  }
}
