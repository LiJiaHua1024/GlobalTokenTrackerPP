import 'package:html/dom.dart' as dom;
import 'package:html/parser.dart' as html_parser;

/// Converts HTML tags and mixed HTML/Markdown in release notes into clean,
/// standardized Markdown that can be rendered seamlessly by [MarkdownBody].
class ChangelogHtmlConverter {
  ChangelogHtmlConverter._();

  /// Converts [input] containing HTML tags or mixed HTML+Markdown into Markdown.
  static String toMarkdown(String input) {
    if (input.trim().isEmpty) return '';

    // Step 1: Protect fenced code blocks (``` or ````) and inline code (`...`)
    final codeBlocks = <String>[];
    String processed = input.replaceAllMapped(
      RegExp(r'(`{3,}[\s\S]*?`{3,}|`[^`\n]+`)'),
      (match) {
        final placeholder = '@@@CHANGELOG_CODE_${codeBlocks.length}@@@';
        codeBlocks.add(match.group(0)!);
        return placeholder;
      },
    );

    // Step 2: Strip HTML comments and script/style tags
    processed = processed.replaceAll(RegExp(r'<!--[\s\S]*?-->'), '');
    processed = processed.replaceAll(
      RegExp(r'<(script|style)[^>]*>[\s\S]*?<\/\1>', caseSensitive: false),
      '',
    );

    // Step 3: Parse GitHub collapsible sections (<details><summary>...</summary>...</details>)
    processed = processed.replaceAllMapped(
      RegExp(r'<details[^>]*>([\s\S]*?)<\/details>', caseSensitive: false),
      (match) {
        final inside = match.group(1)!;
        String summary = '详细变更';
        String content = inside;
        final summaryMatch = RegExp(
          r'<summary[^>]*>([\s\S]*?)<\/summary>',
          caseSensitive: false,
        ).firstMatch(inside);
        if (summaryMatch != null) {
          summary = summaryMatch
              .group(1)!
              .replaceAll(RegExp(r'<[^>]+>'), '')
              .trim();
          content = inside.replaceRange(summaryMatch.start, summaryMatch.end, '');
        }
        return '\n\n**$summary**\n\n$content\n\n';
      },
    );

    // Step 4: Parse with HTML DOM parser
    final fragment = html_parser.parseFragment(processed);
    final buffer = StringBuffer();
    for (final node in fragment.nodes) {
      buffer.write(_nodeToMarkdown(node));
    }

    String result = buffer.toString();

    // Step 5: Restore protected code blocks
    for (int i = 0; i < codeBlocks.length; i++) {
      result = result.replaceAll('@@@CHANGELOG_CODE_$i@@@', codeBlocks[i]);
    }

    // Step 6: Normalize line breaks and clean excessive whitespace
    result = result.replaceAll(RegExp(r'\r\n'), '\n');
    result = result.replaceAll(RegExp(r'\n{3,}'), '\n\n');

    return result.trim();
  }

  static String _nodeToMarkdown(dom.Node node) {
    if (node is dom.Comment) {
      return '';
    }
    if (node is dom.Text) {
      return node.text;
    }
    if (node is dom.Element) {
      final tag = node.localName?.toLowerCase() ?? '';

      // Special handling for ordered lists
      if (tag == 'ol') {
        int idx = 1;
        final buf = StringBuffer('\n');
        for (final child in node.children) {
          if (child.localName?.toLowerCase() == 'li') {
            buf.writeln('$idx. ${_nodeToMarkdown(child).trim()}');
            idx++;
          } else {
            buf.write(_nodeToMarkdown(child));
          }
        }
        return buf.toString();
      }

      // Special handling for HTML tables -> Markdown tables
      if (tag == 'table') {
        final rows = <List<String>>[];
        for (final tr in node.querySelectorAll('tr')) {
          final cells = tr
              .querySelectorAll('th, td')
              .map((c) => _nodeToMarkdown(c).trim().replaceAll('|', '\\|'))
              .toList();
          if (cells.isNotEmpty) {
            rows.add(cells);
          }
        }
        if (rows.isNotEmpty) {
          final colCount =
              rows.map((r) => r.length).reduce((a, b) => a > b ? a : b);
          final buf = StringBuffer('\n\n');
          for (int r = 0; r < rows.length; r++) {
            final row = rows[r];
            while (row.length < colCount) {
              row.add('');
            }
            buf.writeln('| ${row.join(' | ')} |');
            if (r == 0) {
              buf.writeln('| ${List.filled(colCount, '---').join(' | ')} |');
            }
          }
          buf.write('\n');
          return buf.toString();
        }
      }

      final inner = node.nodes.map(_nodeToMarkdown).join('');

      switch (tag) {
        case 'h1':
          return '\n\n# ${inner.trim()}\n\n';
        case 'h2':
          return '\n\n## ${inner.trim()}\n\n';
        case 'h3':
          return '\n\n### ${inner.trim()}\n\n';
        case 'h4':
          return '\n\n#### ${inner.trim()}\n\n';
        case 'h5':
          return '\n\n##### ${inner.trim()}\n\n';
        case 'h6':
          return '\n\n###### ${inner.trim()}\n\n';
        case 'p':
          return '\n\n${inner.trim()}\n\n';
        case 'div':
        case 'section':
        case 'article':
        case 'center':
          return '\n${inner.trim()}\n';
        case 'span':
        case 'font':
          return inner;
        case 'br':
          return '  \n';
        case 'hr':
          return '\n\n---\n\n';
        case 'b':
        case 'strong':
          final t = inner.trim();
          return t.isNotEmpty ? '**$t**' : '';
        case 'i':
        case 'em':
          final t = inner.trim();
          return t.isNotEmpty ? '*$t*' : '';
        case 'del':
        case 's':
        case 'strike':
          final t = inner.trim();
          return t.isNotEmpty ? '~~$t~~' : '';
        case 'code':
          return '`${inner.trim()}`';
        case 'pre':
          return '\n```\n${node.text.trim()}\n```\n';
        case 'blockquote':
          final lines = inner.trim().split('\n');
          return '\n${lines.map((l) => '> $l').join('\n')}\n';
        case 'a':
          final href = node.attributes['href']?.trim() ?? '';
          final text = inner.trim();
          if (href.isNotEmpty) {
            return '[${text.isNotEmpty ? text : href}]($href)';
          }
          return text;
        case 'img':
          final src = node.attributes['src']?.trim() ?? '';
          final alt = node.attributes['alt']?.trim() ?? '';
          if (src.isNotEmpty) {
            return '![$alt]($src)';
          }
          return '';
        case 'ul':
          return '\n$inner\n';
        case 'li':
          return '\n- ${inner.trim()}';
        default:
          return inner;
      }
    }
    return '';
  }
}
