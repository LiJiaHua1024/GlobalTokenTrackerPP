import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../core/ffi_bridge.dart';
import '../core/theme.dart';

class SettingsPage extends StatelessWidget {
  const SettingsPage({Key? key}) : super(key: key);

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
    final theme = Theme.of(context);
    final dbPath = FfiBridge.instance.getDefaultDbPath();

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
                  SegmentedButton<double>(
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

          // Storage Info
          Card(
            child: ListTile(
              contentPadding: const EdgeInsets.all(20),
              leading: Icon(Icons.storage, size: 36, color: theme.colorScheme.primary),
              title: const Text('本地 SQLite 账本文件位置', style: TextStyle(fontWeight: FontWeight.bold)),
              subtitle: Padding(
                padding: const EdgeInsets.only(top: 6.0),
                child: SelectableText(dbPath, style: TextStyle(fontFamily: 'Consolas', color: theme.colorScheme.outline)),
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
                  Text('关于 GlobalTokenTracker++', style: theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.bold)),
                  const SizedBox(height: 8),
                  const Text('基于 Flutter Desktop 与 Google Material Design 3 打造的高性能全息用量仪表盘。'),
                  const SizedBox(height: 6),
                  Text('底层接入 Rust 核心引擎，零网络上报，纯本地高精度分桶计量。', style: TextStyle(color: theme.colorScheme.outline)),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
