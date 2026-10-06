import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';
import '../core/app_info.dart';
import '../core/update_models.dart';
import '../core/update_provider.dart';
import 'changelog_view.dart';

class UpdateDialog extends StatelessWidget {
  final UpdateInfo updateInfo;

  const UpdateDialog({super.key, required this.updateInfo});

  static Future<void> show(BuildContext context, UpdateInfo info) {
    return showDialog(
      context: context,
      barrierDismissible: false,
      builder: (ctx) => UpdateDialog(updateInfo: info),
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;
    final updateProvider = Provider.of<UpdateProvider>(context);

    final isDownloading = updateProvider.downloadStatus == DownloadStatus.downloading;
    final isCompleted = updateProvider.downloadStatus == DownloadStatus.completed;
    final isFailed = updateProvider.downloadStatus == DownloadStatus.failed;

    final asset = updateInfo.setupAsset ?? updateInfo.primaryAsset;

    final dateStr = DateFormat('yyyy-MM-dd').format(updateInfo.publishedAt);

    return Dialog(
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(20)),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 580, maxHeight: 680),
        child: Padding(
          padding: const EdgeInsets.all(24.0),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // Header
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Container(
                    width: 48,
                    height: 48,
                    decoration: BoxDecoration(
                      color: colorScheme.primaryContainer,
                      borderRadius: BorderRadius.circular(12),
                    ),
                    child: Icon(
                      Icons.system_update_alt_rounded,
                      color: colorScheme.onPrimaryContainer,
                      size: 28,
                    ),
                  ),
                  const SizedBox(width: 16),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          '发现新版本可用',
                          style: theme.textTheme.titleLarge?.copyWith(fontWeight: FontWeight.bold),
                        ),
                        const SizedBox(height: 4),
                        Row(
                          children: [
                            Container(
                              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                              decoration: BoxDecoration(
                                color: colorScheme.surfaceContainerHighest,
                                borderRadius: BorderRadius.circular(6),
                              ),
                              child: Text(
                                'v${AppInfo.currentVersion}',
                                style: TextStyle(
                                  fontSize: 12,
                                  color: colorScheme.onSurfaceVariant,
                                  fontWeight: FontWeight.w500,
                                ),
                              ),
                            ),
                            const Padding(
                              padding: EdgeInsets.symmetric(horizontal: 6),
                              child: Icon(Icons.arrow_forward_rounded, size: 14),
                            ),
                            Container(
                              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                              decoration: BoxDecoration(
                                color: colorScheme.primary.withOpacity(0.15),
                                borderRadius: BorderRadius.circular(6),
                              ),
                              child: Text(
                                'v${updateInfo.version}',
                                style: TextStyle(
                                  fontSize: 12,
                                  color: colorScheme.primary,
                                  fontWeight: FontWeight.bold,
                                ),
                              ),
                            ),
                            if (updateInfo.isPrerelease) ...[
                              const SizedBox(width: 6),
                              Container(
                                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                                decoration: BoxDecoration(
                                  color: Colors.amber.withOpacity(0.2),
                                  borderRadius: BorderRadius.circular(4),
                                ),
                                child: const Text(
                                  '预发布',
                                  style: TextStyle(
                                    fontSize: 11,
                                    color: Colors.amber,
                                    fontWeight: FontWeight.bold,
                                  ),
                                ),
                              ),
                            ],
                          ],
                        ),
                      ],
                    ),
                  ),
                  IconButton(
                    onPressed: isDownloading ? null : () => Navigator.of(context).pop(),
                    icon: const Icon(Icons.close),
                    tooltip: '关闭',
                  ),
                ],
              ),
              const SizedBox(height: 12),

              // Metadata info row
              Row(
                children: [
                  Icon(Icons.calendar_today_outlined, size: 14, color: colorScheme.outline),
                  const SizedBox(width: 4),
                  Text('发布于 $dateStr', style: TextStyle(fontSize: 12, color: colorScheme.outline)),
                  if (asset != null) ...[
                    const SizedBox(width: 16),
                    Icon(Icons.folder_zip_outlined, size: 14, color: colorScheme.outline),
                    const SizedBox(width: 4),
                    Text('安装包大小: ${asset.formattedSize}',
                        style: TextStyle(fontSize: 12, color: colorScheme.outline)),
                  ],
                ],
              ),
              const SizedBox(height: 16),

              // Release notes title
              Text(
                '更新日志与更新详情：',
                style: theme.textTheme.bodyMedium?.copyWith(fontWeight: FontWeight.bold),
              ),
              const SizedBox(height: 8),

              // Scrollable Release Notes
              Expanded(
                child: Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(14),
                  decoration: BoxDecoration(
                    color: colorScheme.surfaceContainerHighest.withOpacity(0.4),
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(color: colorScheme.outlineVariant.withOpacity(0.5)),
                  ),
                  child: Scrollbar(
                    thumbVisibility: true,
                    child: SingleChildScrollView(
                      child: ChangelogView(
                        content: updateInfo.releaseNotes.isNotEmpty
                            ? updateInfo.releaseNotes
                            : updateInfo.title,
                      ),
                    ),
                  ),
                ),
              ),
              const SizedBox(height: 16),

              // Download / Installation Progress View
              if (isDownloading) ...[
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Text(
                          '正在下载更新安装包...',
                          style: TextStyle(
                            fontSize: 13,
                            fontWeight: FontWeight.w500,
                            color: colorScheme.primary,
                          ),
                        ),
                        Text(
                          '${(updateProvider.downloadedBytes / (1024 * 1024)).toStringAsFixed(1)} MB / '
                          '${(updateProvider.totalBytes / (1024 * 1024)).toStringAsFixed(1)} MB '
                          '(${(updateProvider.downloadProgress * 100).toInt()}%)',
                          style: TextStyle(fontSize: 12, color: colorScheme.outline),
                        ),
                      ],
                    ),
                    const SizedBox(height: 8),
                    ClipRRect(
                      borderRadius: BorderRadius.circular(4),
                      child: LinearProgressIndicator(
                        value: updateProvider.downloadProgress,
                        minHeight: 6,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 12),
              ] else if (isCompleted) ...[
                Container(
                  padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
                  decoration: BoxDecoration(
                    color: Colors.green.withOpacity(0.12),
                    borderRadius: BorderRadius.circular(8),
                    border: Border.all(color: Colors.green.withOpacity(0.3)),
                  ),
                  child: Row(
                    children: [
                      const Icon(Icons.check_circle_rounded, color: Colors.green, size: 20),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          '新版本安装程序已准备完毕，点击右下角按钮即可自动启动安装向导并更新。',
                          style: TextStyle(color: colorScheme.onSurface, fontSize: 13),
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 12),
              ] else if (isFailed) ...[
                Container(
                  padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
                  decoration: BoxDecoration(
                    color: colorScheme.errorContainer.withOpacity(0.5),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: Row(
                    children: [
                      Icon(Icons.error_outline, color: colorScheme.error, size: 20),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          updateProvider.errorMessage ?? '下载失败，请重试或在浏览器中下载',
                          style: TextStyle(color: colorScheme.onErrorContainer, fontSize: 12),
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 12),
              ],

              // Action buttons bar
              Row(
                children: [
                  TextButton.icon(
                    onPressed: () {
                      final url = updateInfo.htmlUrl.isNotEmpty
                          ? updateInfo.htmlUrl
                          : AppInfo.releasesWebUrl;
                      UpdateProvider.openInBrowser(url);
                    },
                    icon: const Icon(Icons.open_in_new, size: 16),
                    label: const Text('前往发布页'),
                  ),
                  const Spacer(),
                  if (!isDownloading && !isCompleted) ...[
                    TextButton(
                      onPressed: () {
                        updateProvider.skipCurrentVersion();
                        Navigator.of(context).pop();
                      },
                      child: const Text('跳过此版本'),
                    ),
                    const SizedBox(width: 8),
                    OutlinedButton(
                      onPressed: () {
                        updateProvider.dismissForSession();
                        Navigator.of(context).pop();
                      },
                      child: const Text('稍后提醒'),
                    ),
                    const SizedBox(width: 8),
                    FilledButton.icon(
                      onPressed: () {
                        updateProvider.startDownload();
                      },
                      icon: const Icon(Icons.download, size: 18),
                      label: Text(isFailed ? '重新下载' : '立即下载更新'),
                    ),
                  ] else if (isDownloading) ...[
                    OutlinedButton(
                      onPressed: () {
                        updateProvider.cancelDownload();
                      },
                      child: const Text('取消下载'),
                    ),
                  ] else if (isCompleted) ...[
                    FilledButton.icon(
                      onPressed: () {
                        updateProvider.launchInstallerAndExit();
                      },
                      icon: const Icon(Icons.system_update_alt_rounded, size: 18),
                      label: const Text('立即安装并重启'),
                    ),
                  ],
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}
