import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:markdown/markdown.dart' as md;
import '../core/changelog_html_converter.dart';
import '../core/update_provider.dart';

/// Renders changelogs and release notes with full Markdown and HTML tag support.
///
/// Features:
/// - Strips / converts raw HTML tags (`<div>`, `<p>`, `<b>`, `<a>`, `<details>`, etc.)
///   into clean, standard Markdown.
/// - Full GitHub Flavored Markdown (GFM) support including tables, task lists, code blocks,
///   strikethrough, and links.
/// - Integrated Material Design 3 styling consistent with the app theme.
/// - Supports selectable text and clickable external URLs.
class ChangelogView extends StatelessWidget {
  final String content;
  final EdgeInsetsGeometry? padding;
  final bool selectable;

  const ChangelogView({
    super.key,
    required this.content,
    this.padding,
    this.selectable = true,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    final trimmed = content.trim();
    if (trimmed.isEmpty) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24.0),
          child: Text(
            '暂无更新日志详情',
            style: TextStyle(
              color: colorScheme.outline,
              fontSize: 13,
            ),
          ),
        ),
      );
    }

    final markdown = ChangelogHtmlConverter.toMarkdown(trimmed);

    final styleSheet = MarkdownStyleSheet.fromTheme(theme).copyWith(
      p: theme.textTheme.bodyMedium?.copyWith(
        height: 1.6,
        color: colorScheme.onSurface,
      ),
      h1: theme.textTheme.titleLarge?.copyWith(
        fontWeight: FontWeight.bold,
        color: colorScheme.onSurface,
        height: 1.8,
      ),
      h2: theme.textTheme.titleMedium?.copyWith(
        fontWeight: FontWeight.bold,
        color: colorScheme.onSurface,
        height: 1.7,
      ),
      h3: theme.textTheme.titleSmall?.copyWith(
        fontWeight: FontWeight.bold,
        color: colorScheme.primary,
        height: 1.6,
      ),
      h4: theme.textTheme.bodyLarge?.copyWith(
        fontWeight: FontWeight.bold,
        color: colorScheme.onSurface,
        height: 1.5,
      ),
      h5: theme.textTheme.bodyMedium?.copyWith(
        fontWeight: FontWeight.bold,
        color: colorScheme.onSurface,
      ),
      h6: theme.textTheme.bodySmall?.copyWith(
        fontWeight: FontWeight.bold,
        color: colorScheme.onSurface,
      ),
      listBullet: theme.textTheme.bodyMedium?.copyWith(
        color: colorScheme.primary,
        fontWeight: FontWeight.bold,
      ),
      code: TextStyle(
        fontFamily: 'Consolas',
        fontSize: 12.5,
        color: colorScheme.onSurfaceVariant,
        backgroundColor: colorScheme.surfaceContainerHighest.withOpacity(0.6),
      ),
      codeblockDecoration: BoxDecoration(
        color: colorScheme.surfaceContainerHighest.withOpacity(0.45),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(
          color: colorScheme.outlineVariant.withOpacity(0.5),
        ),
      ),
      codeblockPadding: const EdgeInsets.all(12),
      blockquote: theme.textTheme.bodyMedium?.copyWith(
        color: colorScheme.onSurfaceVariant,
        fontStyle: FontStyle.italic,
        height: 1.5,
      ),
      blockquoteDecoration: BoxDecoration(
        color: colorScheme.primaryContainer.withOpacity(0.12),
        borderRadius: BorderRadius.circular(6),
        border: Border(
          left: BorderSide(color: colorScheme.primary, width: 3.5),
        ),
      ),
      blockquotePadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
      a: TextStyle(
        color: colorScheme.primary,
        decoration: TextDecoration.underline,
        fontWeight: FontWeight.w500,
      ),
      tableBorder: TableBorder.all(
        color: colorScheme.outlineVariant.withOpacity(0.5),
        width: 1,
      ),
      tableHead: theme.textTheme.bodyMedium?.copyWith(
        fontWeight: FontWeight.bold,
        color: colorScheme.onSurface,
      ),
      tableBody: theme.textTheme.bodyMedium?.copyWith(
        color: colorScheme.onSurface,
      ),
      tablePadding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      horizontalRuleDecoration: BoxDecoration(
        border: Border(
          top: BorderSide(
            color: colorScheme.outlineVariant.withOpacity(0.6),
            width: 1,
          ),
        ),
      ),
    );

    final markdownBody = MarkdownBody(
      data: markdown,
      selectable: selectable,
      extensionSet: md.ExtensionSet.gitHubFlavored,
      styleSheet: styleSheet,
      onTapLink: (text, href, title) {
        if (href != null && href.trim().isNotEmpty) {
          UpdateProvider.openInBrowser(href.trim());
        }
      },
    );

    if (padding != null) {
      return Padding(
        padding: padding!,
        child: markdownBody,
      );
    }

    return markdownBody;
  }
}
