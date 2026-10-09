import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../../core/models.dart';
import '../../core/theme.dart';

class QuotaCardWidget extends StatelessWidget {
  final QuotaGroup group;

  const QuotaCardWidget({Key? key, required this.group}) : super(key: key);

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final themeProvider = Provider.of<ThemeProvider>(context);

    return Card(
      elevation: 1,
      child: Padding(
        padding: const EdgeInsets.all(18.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Row(
                  children: [
                    Icon(Icons.shield_outlined, size: 20, color: theme.colorScheme.primary),
                    const SizedBox(width: 8),
                    Text(
                      group.display,
                      style: theme.textTheme.titleMedium?.copyWith(
                        fontWeight: FontWeight.bold,
                      ),
                    ),
                  ],
                ),
                if (group.worstPct != null)
                  Builder(builder: (context) {
                    final worstVal = group.worstPct!;
                    final displayPct = worstVal > 1.0 ? worstVal : (worstVal * 100.0);
                    final colorPct = worstVal > 1.0 ? (worstVal / 100.0) : worstVal;
                    return Container(
                      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                      decoration: BoxDecoration(
                        color: _getPctColor(colorPct, theme).withValues(alpha: 0.15),
                        borderRadius: BorderRadius.circular(12),
                      ),
                      child: Text(
                        '已用 ${displayPct.toStringAsFixed(1)}%',
                        style: TextStyle(
                          fontSize: 12,
                          fontWeight: FontWeight.bold,
                          color: _getPctColor(colorPct, theme),
                        ),
                      ),
                    );
                  }),
              ],
            ),
            const SizedBox(height: 12),
            ...group.rows.map((row) {
              final rawPct = row.usedPercent ??
                  (row.used != null && row.limitValue != null && row.limitValue! > 0
                      ? (row.used! / row.limitValue!)
                      : 0.0);
              final normalizedPct = rawPct > 1.0 ? (rawPct / 100.0) : rawPct;
              final pctClamped = normalizedPct.clamp(0.0, 1.0);

              return Padding(
                padding: const EdgeInsets.symmetric(vertical: 4.0),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Text(
                          _formatWindowKind(row.windowKind),
                          style: theme.textTheme.bodyMedium?.copyWith(
                            fontWeight: FontWeight.w500,
                          ),
                        ),
                        Text(
                          row.used != null && row.limitValue != null
                              ? '${themeProvider.formatTokens(row.used!)} / ${themeProvider.formatTokens(row.limitValue!)}'
                              : (row.used != null ? themeProvider.formatTokens(row.used!) : '未知'),
                          style: theme.textTheme.bodySmall?.copyWith(
                            color: theme.colorScheme.outline,
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 4),
                    ClipRRect(
                      borderRadius: BorderRadius.circular(4),
                      child: LinearProgressIndicator(
                        value: pctClamped,
                        minHeight: 6,
                        backgroundColor: theme.colorScheme.surfaceContainerHighest,
                        valueColor: AlwaysStoppedAnimation<Color>(_getPctColor(normalizedPct, theme)),
                      ),
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

  String _formatWindowKind(String kind) {
    switch (kind) {
      case '5h_block':
        return '5 小时窗口';
      case 'weekly':
        return '每周限额';
      case 'monthly':
        return '每月限额';
      case 'daily':
        return '每日限额';
      case 'credits':
        return '剩余点数';
      case 'billing_period':
        return '计费周期';
      case 'auto_pool':
        return 'Auto 用量池';
      case 'api_pool':
        return 'API 用量池';
      case 'session_ctx':
        return '会话上下文';
      default:
        return kind;
    }
  }

  Color _getPctColor(double pct, ThemeData theme) {
    if (pct >= 0.9) return theme.colorScheme.error;
    if (pct >= 0.75) return Colors.orange;
    return theme.colorScheme.primary;
  }
}
