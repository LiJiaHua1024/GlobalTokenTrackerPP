import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/changelog_html_converter.dart';
import 'package:globaltokentracker_ui/core/update_models.dart';
import 'package:globaltokentracker_ui/core/update_provider.dart';
import 'package:globaltokentracker_ui/widgets/changelog_view.dart';
import 'package:globaltokentracker_ui/widgets/update_dialog.dart';
import 'package:provider/provider.dart';

void main() {
  group('ChangelogHtmlConverter tests', () {
    test('Pure markdown is preserved', () {
      const input = '''
### 🚀 Features
- Item 1
- Item 2
  - Subitem 2.1

```dart
final a = 1;
```
''';
      final md = ChangelogHtmlConverter.toMarkdown(input);
      expect(md.contains('### 🚀 Features'), isTrue);
      expect(md.contains('- Item 1'), isTrue);
      expect(md.contains('final a = 1;'), isTrue);
    });

    test('Converts HTML div, p, b, i, a, br tags to markdown', () {
      const input = '''
<div align="center">
  <h2>GlobalTokenTracker++ v1.2.0</h2>
  <p>一个轻量且强大的 <b>Token</b> 追踪工具</p>
</div>
<div class="list">
  <ul>
    <li>支持自动更新检测 (<a href="https://example.com/pr/1">#1</a>)</li>
    <li><i>斜体强调文本</i></li>
  </ul>
</div>
<p>换行测试<br/>第二行</p>
''';
      final md = ChangelogHtmlConverter.toMarkdown(input);

      expect(md.contains('## GlobalTokenTracker++ v1.2.0'), isTrue);
      expect(md.contains('**Token**'), isTrue);
      expect(md.contains('[#1](https://example.com/pr/1)'), isTrue);
      expect(md.contains('*斜体强调文本*'), isTrue);
      expect(md.contains('换行测试  \n第二行'), isTrue);

      // Verify no div or raw HTML tags remain
      expect(md.contains('<div'), isFalse);
      expect(md.contains('</div>'), isFalse);
      expect(md.contains('<p>'), isFalse);
      expect(md.contains('</p>'), isFalse);
      expect(md.contains('<b>'), isFalse);
      expect(md.contains('<br'), isFalse);
    });

    test('Converts HTML table and details to markdown', () {
      const input = '''
<details>
<summary>详细更新信息</summary>
<table>
  <tr><th>模块</th><th>状态</th></tr>
  <tr><td>核心</td><td>已优化</td></tr>
</table>
</details>
''';
      final md = ChangelogHtmlConverter.toMarkdown(input);

      expect(md.contains('**详细更新信息**'), isTrue);
      expect(md.contains('| 模块 | 状态 |'), isTrue);
      expect(md.contains('| --- | --- |'), isTrue);
      expect(md.contains('| 核心 | 已优化 |'), isTrue);
      expect(md.contains('<details>'), isFalse);
      expect(md.contains('<table>'), isFalse);
    });

    test('Code blocks containing HTML-like syntax are protected', () {
      const input = '''
```xml
<div>protected content</div>
```
Inline `<div>inline code</div>` should be protected.
<!-- comment to be removed -->
''';
      final md = ChangelogHtmlConverter.toMarkdown(input);

      expect(md.contains('<div>protected content</div>'), isTrue);
      expect(md.contains('`<div>inline code</div>`'), isTrue);
      expect(md.contains('comment to be removed'), isFalse);
    });

    test('Empty or blank input returns empty string', () {
      expect(ChangelogHtmlConverter.toMarkdown(''), '');
      expect(ChangelogHtmlConverter.toMarkdown('   \n  '), '');
    });
  });

  group('ChangelogView widget tests', () {
    testWidgets('Renders empty state placeholder when content is empty', (tester) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: ChangelogView(content: ''),
          ),
        ),
      );

      expect(find.text('暂无更新日志详情'), findsOneWidget);
    });

    testWidgets('Renders markdown and parsed HTML without raw div tags', (tester) async {
      const changelog = '''
<div align="center">
  <h3>🎉 版本 1.6.0 发布</h3>
</div>
### 🌟 新特性
<div>
- 增强 Token 统计分析
- 优化系统托盘右键菜单
</div>
<p>相关链接: <a href="https://github.com/test">查看详情</a></p>
''';

      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: ChangelogView(content: changelog),
          ),
        ),
      );

      expect(find.textContaining('版本 1.6.0 发布'), findsOneWidget);
      expect(find.textContaining('新特性'), findsOneWidget);
      expect(find.textContaining('增强 Token 统计分析'), findsOneWidget);
      expect(find.textContaining('优化系统托盘右键菜单'), findsOneWidget);
      expect(find.textContaining('查看详情'), findsOneWidget);

      // Verify that literal '<div>' or '</div>' does not appear anywhere in the rendered UI
      expect(find.textContaining('<div>'), findsNothing);
      expect(find.textContaining('</div>'), findsNothing);
      expect(find.textContaining('<div align'), findsNothing);
      expect(find.textContaining('<p>'), findsNothing);
    });
  });

  group('UpdateDialog integration tests with ChangelogView', () {
    testWidgets('UpdateDialog parses and renders HTML/Markdown in releaseNotes', (tester) async {
      final info = UpdateInfo(
        version: '1.2.0',
        tagName: 'v1.2.0',
        title: 'GlobalTokenTracker++ v1.2.0',
        releaseNotes: '''
<div align="center">
  <h3>✨ 新版更新亮点</h3>
</div>

### 🚀 重点改进
<div>
- <b>解析引擎</b>: 完美支持 Markdown 与 HTML 格式日志
- <b>UI 体验</b>: 告别裸文本与 div 标签展示
</div>

<details>
<summary>完整修改日志</summary>
- 修复换行与列表排版
</details>
''',
        publishedAt: DateTime(2026, 10, 1),
        isPrerelease: false,
        htmlUrl: 'https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/releases',
        setupAsset: const ReleaseAsset(
          name: 'GlobalTokenTrackerPP-Setup-1.2.0-win-x64.exe',
          size: 16000000,
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

      // Verify title & version
      expect(find.text('发现新版本可用'), findsOneWidget);
      expect(find.text('v1.2.0'), findsOneWidget);

      // Verify release notes rendered content
      expect(find.textContaining('新版更新亮点'), findsOneWidget);
      expect(find.textContaining('重点改进'), findsOneWidget);
      expect(find.textContaining('解析引擎'), findsOneWidget);
      expect(find.textContaining('UI 体验'), findsOneWidget);
      expect(find.textContaining('完整修改日志'), findsOneWidget);

      // Verify no raw div tags displayed
      expect(find.textContaining('<div>'), findsNothing);
      expect(find.textContaining('</div>'), findsNothing);
      expect(find.textContaining('<div align'), findsNothing);
    });
  });
}
