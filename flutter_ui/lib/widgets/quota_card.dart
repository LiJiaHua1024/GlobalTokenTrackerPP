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
                  Container(
                    padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                    decoration: BoxDecoration(
                      color: _getPctColor(group.worstPct!, theme).withValues(alpha: 0.15),
                      borderRadius: BorderRadius.circular(12),
                    ),
                    child: Text(
                      '已用 ${(group.worstPct! * 100).toStringAsFixed(1)}%',
                      style: TextStyle(
                        fontSize: 12,
                        fontWeight: FontWeight.bold,
                        color: _getPctColor(group.worstPct!, theme),
                      ),
                    ),
                  ),
              ],
            ),
            const SizedBox(height: 14),
            ...group.rows.map((row) {
              final pct = row.usedPercent ?? (row.used != null && row.limitValue != null && row.limitValue! > 0 ? (row.used! / row.limitValue!) : 0.0);
              final pctClamped = pct.clamp(0.0, 1.0);

              return Padding(
                padding: const EdgeInsets.symmetric(vertical: 6.0),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Text(
                          row.windowKind,
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
                    const SizedBox(height: 6),
                    ClipRRect(
                      borderRadius: BorderRadius.circular(4),
                      child: LinearProgressIndicator(
                        value: pctClamped,
                        minHeight: 8,
                        backgroundColor: theme.colorScheme.surfaceVariant,
                        valueColor: AlwaysStoppedAnimation<Color>(_getPctColor(pct, theme)),
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

  Color _getPctColor(double pct, ThemeData theme) {
    if (pct >= 0.9) return theme.colorScheme.error;
    if (pct >= 0.75) return Colors.orange;
    return theme.colorScheme.primary;
  }
}
