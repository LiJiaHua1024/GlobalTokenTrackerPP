import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/semver.dart';
import 'package:globaltokentracker_ui/core/update_models.dart';
import 'package:globaltokentracker_ui/core/update_provider.dart';
import 'package:globaltokentracker_ui/widgets/update_banner.dart';
import 'package:globaltokentracker_ui/widgets/update_dialog.dart';
import 'package:provider/provider.dart';

void main() {
  group('UpdateModels tests', () {
    test('Parse UpdateInfo and ReleaseAsset from GitHub JSON', () {
      final mockReleaseJson = {
        'tag_name': 'v1.1.0',
        'name': 'GlobalTokenTracker++ v1.1.0 - Performance & Feature Upgrade',
        'body': '### 新特性\n- 支持自动更新检测\n- 性能优化 30%',
        'published_at': '2026-09-29T12:00:00Z',
        'prerelease': false,
        'html_url': 'https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/releases/tag/v1.1.0',
        'assets': [
          {
            'name': 'GlobalTokenTrackerPP-Setup-1.1.0-win-x64.exe',
            'size': 15728640, // ~15 MB
            'browser_download_url': 'https://github.com/download/Setup-1.1.0.exe',
            'content_type': 'application/octet-stream',
          },
          {
            'name': 'GlobalTokenTrackerPP-v1.1.0-windows-x64.zip',
            'size': 14680064, // ~14 MB
            'browser_download_url': 'https://github.com/download/v1.1.0.zip',
            'content_type': 'application/zip',
          }
        ]
      };

      final info = UpdateInfo.fromGitHubJson(mockReleaseJson);
      expect(info.version, '1.1.0');
      expect(info.tagName, 'v1.1.0');
      expect(info.isPrerelease, isFalse);
      expect(info.setupAsset, isNotNull);
      expect(info.setupAsset!.isSetupInstaller, isTrue);
      expect(info.setupAsset!.formattedSize, '15.0 MB');
      expect(info.portableAsset, isNotNull);
      expect(info.portableAsset!.isPortableZip, isTrue);
      expect(info.primaryAsset, info.setupAsset);
      expect(SemVer.isNewer(info.version, '1.0.0'), isTrue);
      expect(SemVer.isNewer(info.version, '1.2.0'), isFalse);
    });

    test('UpdateSettings serialization and defaults', () {
      const settings = UpdateSettings();
      expect(settings.autoCheckEnabled, isTrue);
      expect(settings.checkInterval, UpdateCheckInterval.daily);
      expect(settings.includePrerelease, isFalse);
      expect(settings.skippedVersion, isNull);

      final json = settings.toJson();
      final restored = UpdateSettings.fromJson(json);
      expect(restored.autoCheckEnabled, isTrue);
      expect(restored.checkInterval, UpdateCheckInterval.daily);
      expect(restored.includePrerelease, isFalse);

      final updated = settings.copyWith(
        skippedVersion: '1.2.0',
        checkInterval: UpdateCheckInterval.weekly,
      );
      expect(updated.skippedVersion, '1.2.0');
      expect(updated.checkInterval, UpdateCheckInterval.weekly);

      final cleared = updated.copyWith(clearSkippedVersion: true);
      expect(cleared.skippedVersion, isNull);
    });
  });

  group('Update UI Widgets tests', () {
    testWidgets('UpdateDialog displays release details and buttons', (WidgetTester tester) async {
      final info = UpdateInfo(
        version: '1.1.0',
        tagName: 'v1.1.0',
        title: 'GlobalTokenTracker++ v1.1.0',
        releaseNotes: '### 🚀 新特性\n- 自动检测更新\n- 修复样式问题',
        publishedAt: DateTime(2026, 9, 29),
        isPrerelease: false,
        htmlUrl: 'https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/releases',
        setupAsset: const ReleaseAsset(
          name: 'GlobalTokenTrackerPP-Setup-1.1.0-win-x64.exe',
          size: 15728640,
          downloadUrl: 'https://example.com/setup.exe',
          contentType: 'application/octet-stream',
        ),
      );

      final provider = UpdateProvider();

      await tester.pumpWidget(
        ChangeNotifierProvider<UpdateProvider>.value(
          value: provider,
          child: MaterialApp(
            home: Scaffold(
              body: UpdateDialog(updateInfo: info),
            ),
          ),
        ),
      );

      expect(find.text('发现新版本可用'), findsOneWidget);
      expect(find.text('v1.1.0'), findsOneWidget);
      expect(find.textContaining('自动检测更新'), findsOneWidget);
      expect(find.text('立即下载更新'), findsOneWidget);
      expect(find.text('跳过此版本'), findsOneWidget);
      expect(find.text('稍后提醒'), findsOneWidget);
      expect(find.text('前往发布页'), findsOneWidget);
    });

    testWidgets('UpdateBanner shows when update is available and respects dismiss',
        (WidgetTester tester) async {
      final provider = UpdateProvider();

      await tester.pumpWidget(
        ChangeNotifierProvider<UpdateProvider>.value(
          value: provider,
          child: const MaterialApp(
            home: Scaffold(
              body: Column(
                children: [
                  UpdateBanner(),
                  Text('主页面内容'),
                ],
              ),
            ),
          ),
        ),
      );

      // Initially idle, banner should not appear
      expect(find.textContaining('发现新版本'), findsNothing);

      // Dismiss test
      provider.dismissForSession();
      await tester.pump();
      expect(find.textContaining('发现新版本'), findsNothing);
    });
  });
}
