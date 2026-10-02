import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';
import '../core/app_info.dart';
import '../core/ffi_bridge.dart';
import '../core/theme.dart';
import '../core/update_models.dart';
import '../core/update_provider.dart';
import '../widgets/update_dialog.dart';

class SettingsPage extends StatelessWidget {
  const SettingsPage({Key? key}) : super(key: key);

  // Resolved once per app run — a synchronous FFI call inside build() would
  // re-run on every rebuild of this page.
  static final String _dbPath = FfiBridge.instance.getDefaultDbPath();

  static const List<Map<String, dynamic>> _seedColors = [
    {'name': 'Google Blue', 'color': Color(0xFF1A73E8)},
    {'name': 'Emerald Green', 'color': Color(0xFF34A853)},
    {'name': 'Deep Violet', 'color': Color(0xFF6750A4)},
    {'name': 'Teal Ocean', 'color': Color(0xFF00796B)},
    {'name': 'Amber Orange', 'color': Color(0xFFE65100)},
  ];

  @override
  Widget build(BuildContext context) {
    final themeProvider = Provider.of<ThemeProvider>(context);
    final updateProvider = Provider.of<UpdateProvider>(context);
    final theme = Theme.of(context);

    return Padding(
      padding: const EdgeInsets.all(24.0),
      child: ListView(
        children: [
          Text(
            '设置与外观偏好',
            style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
          ),
          const SizedBox(height: 20),

          // Theme Mode
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20.0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('外观主题模式', style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold)),
                  const SizedBox(height: 12),
                  SegmentedButton<ThemeMode>(
                    segments: const [
                      ButtonSegment(value: ThemeMode.system, icon: Icon(Icons.brightness_auto), label: Text('跟随系统')),
                      ButtonSegment(value: ThemeMode.light, icon: Icon(Icons.light_mode), label: Text('浅色模式')),
                      ButtonSegment(value: ThemeMode.dark, icon: Icon(Icons.dark_mode), label: Text('深色模式')),
                    ],
                    selected: {themeProvider.themeMode},
                    onSelectionChanged: (val) => themeProvider.setThemeMode(val.first),
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),

          // UI Scale Preference
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20.0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [
                      Text(
                        '界面显示与缩放',
                        style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold),
                      ),
                      Text(
                        '当前：${(themeProvider.uiScale * 100).toInt()}%',
                        style: TextStyle(
                          color: theme.colorScheme.primary,
                          fontWeight: FontWeight.bold,
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Text(
                    '调整桌面界面缩放比例，适配 2K / 4K 高分辨率与不同屏幕尺寸：',
                    style: TextStyle(color: theme.colorScheme.outline),
                  ),
                  const SizedBox(height: 16),
                  SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: SegmentedButton<double>(
                      segments: const [
                        ButtonSegment(value: 0.85, label: Text('85% 紧凑')),
                        ButtonSegment(value: 1.0, label: Text('100% 默认')),
                        ButtonSegment(value: 1.15, label: Text('115% 适中')),
                        ButtonSegment(value: 1.25, label: Text('125% 大号')),
                        ButtonSegment(value: 1.40, label: Text('140% 超大')),
                      ],
                      selected: {
                        [0.85, 1.0, 1.15, 1.25, 1.40].contains(
                                double.parse(themeProvider.uiScale.toStringAsFixed(2)))
                            ? double.parse(themeProvider.uiScale.toStringAsFixed(2))
                            : 1.0
                      },
                      onSelectionChanged: (val) => themeProvider.setUiScale(val.first),
                    ),
                  ),
                  const SizedBox(height: 12),
                  Row(
                    children: [
                      const Icon(Icons.zoom_out, size: 20),
                      Expanded(
                        child: Slider(
                          value: themeProvider.uiScale.clamp(0.80, 1.50),
                          min: 0.80,
                          max: 1.50,
                          divisions: 14,
                          label: '${(themeProvider.uiScale * 100).toInt()}%',
                          onChanged: (val) => themeProvider.setUiScale(double.parse(val.toStringAsFixed(2))),
                        ),
                      ),
                      const Icon(Icons.zoom_in, size: 20),
                      const SizedBox(width: 8),
                      TextButton.icon(
                        onPressed: themeProvider.uiScale == 1.0
                            ? null
                            : () => themeProvider.setUiScale(1.0),
                        icon: const Icon(Icons.restart_alt, size: 18),
                        label: const Text('重置 100%'),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),

          // Number Display Preference (K, M, B, T)
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20.0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('数值显示偏好', style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold)),
                  const SizedBox(height: 8),
                  Text('针对大额 Token 及用量数值的展示方式：', style: TextStyle(color: theme.colorScheme.outline)),
                  const SizedBox(height: 8),
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('紧凑大数字格式 (K / M / B / T)', style: TextStyle(fontWeight: FontWeight.w500)),
                    subtitle: Text(
                      themeProvider.compactNumbers
                          ? '当前：自动缩写（如 5.79M, 1.20B, 350.2K），界面紧凑优雅'
                          : '当前：完整数字（如 5,790,000），使用千分位精确显示',
                      style: TextStyle(color: theme.colorScheme.outline, fontSize: 13),
                    ),
                    value: themeProvider.compactNumbers,
                    onChanged: (val) => themeProvider.setCompactNumbers(val),
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),

          // Seed Accent Color
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20.0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('Material 3 核心强调色', style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold)),
                  const SizedBox(height: 8),
                  Text('选择应用全局 Tonal Palette 衍生色彩基调：', style: TextStyle(color: theme.colorScheme.outline)),
                  const SizedBox(height: 16),
                  Wrap(
                    spacing: 12,
                    children: _seedColors.map((item) {
                      final color = item['color'] as Color;
                      final isSelected = themeProvider.seedColor == color;

                      return InkWell(
                        onTap: () => themeProvider.setSeedColor(color),
                        borderRadius: BorderRadius.circular(24),
                        child: Container(
                          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
                          decoration: BoxDecoration(
                            color: isSelected ? color.withOpacity(0.15) : Colors.transparent,
                            border: Border.all(
                              color: isSelected ? color : theme.colorScheme.outlineVariant,
                              width: isSelected ? 2 : 1,
                            ),
                            borderRadius: BorderRadius.circular(24),
                          ),
                          child: Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              CircleAvatar(backgroundColor: color, radius: 8),
                              const SizedBox(width: 8),
                              Text(item['name'], style: TextStyle(fontWeight: isSelected ? FontWeight.bold : FontWeight.normal)),
                            ],
                          ),
                        ),
                      );
                    }).toList(),
                  ),
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),

          // Software Update & Maintenance Card
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20.0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [
                      Row(
                        children: [
                          Icon(Icons.system_update_rounded, color: theme.colorScheme.primary, size: 22),
                          const SizedBox(width: 8),
                          Text(
                            '软件更新与检测',
                            style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold),
                          ),
                        ],
                      ),
                      Container(
                        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                        decoration: BoxDecoration(
                          color: theme.colorScheme.primaryContainer,
                          borderRadius: BorderRadius.circular(12),
                        ),
                        child: Text(
                          '当前版本: v${AppInfo.currentVersion}',
                          style: TextStyle(
                            fontSize: 12,
                            fontWeight: FontWeight.bold,
                            color: theme.colorScheme.onPrimaryContainer,
                          ),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Text(
                    '支持手动检测与后台定时检测 GitHub 官方发布的最新版本与功能改进：',
                    style: TextStyle(color: theme.colorScheme.outline),
                  ),
                  const SizedBox(height: 16),

                  // Update Status Feedback Box
                  _buildUpdateStatusBox(context, theme, updateProvider),
                  const SizedBox(height: 16),

                  // Auto Check Switch
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('自动检测更新', style: TextStyle(fontWeight: FontWeight.w500)),
                    subtitle: Text(
                      '在软件启动及后台运行期间自动检测新版本，保持应用处于最佳状态',
                      style: TextStyle(color: theme.colorScheme.outline, fontSize: 13),
                    ),
                    value: updateProvider.settings.autoCheckEnabled,
                    onChanged: (val) => updateProvider.setAutoCheck(val),
                  ),
                  const Divider(height: 24),

                  // Check Interval SegmentedButton
                  if (updateProvider.settings.autoCheckEnabled) ...[
                    Wrap(
                      alignment: WrapAlignment.spaceBetween,
                      crossAxisAlignment: WrapCrossAlignment.center,
                      spacing: 16,
                      runSpacing: 12,
                      children: [
                        Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            const Text('自动检测频率', style: TextStyle(fontWeight: FontWeight.w500)),
                            const SizedBox(height: 4),
                            Text(
                              '设定后台检测新版本的周期间隔',
                              style: TextStyle(color: theme.colorScheme.outline, fontSize: 13),
                            ),
                          ],
                        ),
                        SingleChildScrollView(
                          scrollDirection: Axis.horizontal,
                          child: SegmentedButton<UpdateCheckInterval>(
                            segments: const [
                              ButtonSegment(
                                value: UpdateCheckInterval.onStartup,
                                label: Text('每次启动'),
                              ),
                              ButtonSegment(
                                value: UpdateCheckInterval.daily,
                                label: Text('每天一次'),
                              ),
                              ButtonSegment(
                                value: UpdateCheckInterval.weekly,
                                label: Text('每周一次'),
                              ),
                            ],
                            selected: {updateProvider.settings.checkInterval},
                            onSelectionChanged: (val) => updateProvider.setCheckInterval(val.first),
                          ),
                        ),
                      ],
                    ),
                    const Divider(height: 24),
                  ],

                  // Pre-release Channel Toggle
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    title: const Text(
                      '接收预发布预览版本 (Alpha / Pre-release)',
                      style: TextStyle(fontWeight: FontWeight.w500),
                    ),
                    subtitle: Text(
                      '提前体验最新开发功能与前沿优化（可能存在未完全稳定的改动）',
                      style: TextStyle(color: theme.colorScheme.outline, fontSize: 13),
                    ),
                    value: updateProvider.settings.includePrerelease,
                    onChanged: (val) => updateProvider.setIncludePrerelease(val),
                  ),

                  // Skipped Version notice if any
                  if (updateProvider.settings.skippedVersion != null) ...[
                    const Divider(height: 24),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Row(
                          children: [
                            Icon(Icons.info_outline, size: 18, color: theme.colorScheme.outline),
                            const SizedBox(width: 8),
                            Text(
                              '已忽略版本更新: v${updateProvider.settings.skippedVersion}',
                              style: TextStyle(color: theme.colorScheme.outline, fontSize: 13),
                            ),
                          ],
                        ),
                        TextButton(
                          onPressed: () => updateProvider.clearSkippedVersion(),
                          child: const Text('恢复提示'),
                        ),
                      ],
                    ),
                  ],
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),

          // Storage Info
          Card(
            child: ListTile(
              contentPadding: const EdgeInsets.all(20),
              leading: Icon(Icons.storage, size: 36, color: theme.colorScheme.primary),
              title: const Text('本地 SQLite 账本文件位置', style: TextStyle(fontWeight: FontWeight.bold)),
              subtitle: Padding(
                padding: const EdgeInsets.only(top: 6.0),
                child: SelectableText(_dbPath, style: TextStyle(fontFamily: 'Consolas', color: theme.colorScheme.outline)),
              ),
            ),
          ),
          const SizedBox(height: 16),

          // About GlobalTokenTracker++
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20.0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [
                      Text('关于 GlobalTokenTracker++', style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold)),
                      OutlinedButton.icon(
                        onPressed: () => UpdateProvider.openInBrowser(AppInfo.releasesWebUrl),
                        icon: const Icon(Icons.code, size: 16),
                        label: const Text('GitHub 仓库'),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  const Text('基于 Flutter Desktop 与 Google Material Design 3 打造的高性能全息用量仪表盘。'),
                  const SizedBox(height: 6),
                  Text(
                    '版本: v${AppInfo.currentVersion} (${AppInfo.platform} ${AppInfo.architecture}) · 纯本地高精度分桶计量，零网络隐私上报。',
                    style: TextStyle(color: theme.colorScheme.outline),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildUpdateStatusBox(
    BuildContext context,
    ThemeData theme,
    UpdateProvider updateProvider,
  ) {
    final colorScheme = theme.colorScheme;
    final lastCheck = updateProvider.settings.lastCheckTime;
    final lastCheckStr = lastCheck != null
        ? DateFormat('yyyy-MM-dd HH:mm').format(lastCheck)
        : '从未检测';

    if (updateProvider.isChecking) {
      return Container(
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: colorScheme.surfaceContainerHighest.withOpacity(0.5),
          borderRadius: BorderRadius.circular(12),
        ),
        child: Row(
          children: [
            const SizedBox(
              width: 18,
              height: 18,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
            const SizedBox(width: 12),
            Text(
              '正在连接 GitHub 检查最新版本...',
              style: TextStyle(fontSize: 13, color: colorScheme.onSurfaceVariant),
            ),
          ],
        ),
      );
    }

    if (updateProvider.hasUpdate && updateProvider.updateInfo != null) {
      final info = updateProvider.updateInfo!;
      return Container(
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          color: colorScheme.primaryContainer.withOpacity(0.5),
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: colorScheme.primary.withOpacity(0.4)),
        ),
        child: Row(
          children: [
            Icon(Icons.new_releases_rounded, color: colorScheme.primary, size: 28),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    '发现新版本: v${info.version}',
                    style: const TextStyle(fontWeight: FontWeight.bold, fontSize: 14),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    '发布于 ${DateFormat('yyyy-MM-dd').format(info.publishedAt)} · 点击查看详情并进行一键更新',
                    style: TextStyle(fontSize: 12, color: colorScheme.outline),
                  ),
                ],
              ),
            ),
            FilledButton.icon(
              onPressed: () => UpdateDialog.show(context, info),
              icon: const Icon(Icons.arrow_forward, size: 16),
              label: const Text('查看并更新'),
            ),
          ],
        ),
      );
    }

    if (updateProvider.status == UpdateStatus.error) {
      return Container(
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: colorScheme.errorContainer.withOpacity(0.5),
          borderRadius: BorderRadius.circular(12),
        ),
        child: Row(
          children: [
            Icon(Icons.warning_amber_rounded, color: colorScheme.error, size: 24),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    '检查更新遇到问题',
                    style: TextStyle(
                      fontWeight: FontWeight.bold,
                      fontSize: 13,
                      color: colorScheme.onErrorContainer,
                    ),
                  ),
                  Text(
                    updateProvider.errorMessage ?? '无法连接到更新服务器',
                    style: TextStyle(fontSize: 12, color: colorScheme.onErrorContainer.withOpacity(0.8)),
                  ),
                ],
              ),
            ),
            FilledButton.tonalIcon(
              onPressed: () => updateProvider.checkForUpdates(force: true),
              icon: const Icon(Icons.refresh, size: 16),
              label: const Text('重试'),
            ),
          ],
        ),
      );
    }

    // Default / Up to date state
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerHighest.withOpacity(0.4),
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: colorScheme.outlineVariant.withOpacity(0.5)),
      ),
      child: Row(
        children: [
          Icon(
            updateProvider.status == UpdateStatus.upToDate
                ? Icons.check_circle_rounded
                : Icons.update_rounded,
            color: updateProvider.status == UpdateStatus.upToDate
                ? Colors.green
                : colorScheme.outline,
            size: 24,
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  updateProvider.status == UpdateStatus.upToDate
                      ? '当前已是最新版本 (v${AppInfo.currentVersion})'
                      : '保持软件为最新状态',
                  style: const TextStyle(fontWeight: FontWeight.bold, fontSize: 13),
                ),
                const SizedBox(height: 2),
                Text(
                  '上次检测时间: $lastCheckStr',
                  style: TextStyle(fontSize: 12, color: colorScheme.outline),
                ),
              ],
            ),
          ),
          OutlinedButton.icon(
            onPressed: () async {
              final info = await updateProvider.checkForUpdates(force: true);
              if (info != null && context.mounted) {
                UpdateDialog.show(context, info);
              }
            },
            icon: const Icon(Icons.refresh, size: 16),
            label: const Text('立即检查更新'),
          ),
        ],
      ),
    );
  }
}
