import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:globaltokentracker_ui/core/models.dart';
import 'package:globaltokentracker_ui/core/theme.dart';
import 'package:globaltokentracker_ui/widgets/quota_card.dart';

Widget _wrap(QuotaGroup group) {
  return ChangeNotifierProvider(
    create: (_) => ThemeProvider(),
    child: MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: QuotaCardWidget(group: group),
        ),
      ),
    ),
  );
}

QuotaGroup _group(double percent) => QuotaGroup(
      app: 'codex',
      display: 'Codex',
      worstPct: percent,
      rows: [
        QuotaRow(
          app: 'codex',
          capturedAt: 0,
          windowKind: 'weekly',
          usedPercent: percent,
        ),
      ],
    );

void main() {
  testWidgets('expired windows are labeled and do not show an active limit badge', (tester) async {
    final group=QuotaGroup(app:'codex',display:'Codex',worstPct:99,rows:[QuotaRow(app:'codex',capturedAt:0,windowKind:'weekly',usedPercent:99,resetsAt:1)]);
    await tester.pumpWidget(_wrap(group));
    expect(find.text('已过期'),findsOneWidget);
    expect(find.text('已用 99.0%'),findsNothing);
    expect(find.text('每周限额 · 已过期'),findsOneWidget);
  });
  testWidgets('renders 1% as 1%, not 100%', (tester) async {
    await tester.pumpWidget(_wrap(_group(1.0)));

    expect(find.text('已用 1.0%'), findsOneWidget);
    expect(find.text('已用 100.0%'), findsNothing);
  });

  testWidgets('renders a sub-1% value without scaling it up', (tester) async {
    await tester.pumpWidget(_wrap(_group(0.5)));

    expect(find.text('已用 0.5%'), findsOneWidget);
    expect(find.text('已用 50.0%'), findsNothing);
  });

  testWidgets('derives percent from used/limit when used_percent is absent',
      (tester) async {
    final group = QuotaGroup(
      app: 'cursor',
      display: 'Cursor',
      worstPct: null,
      rows: [
        QuotaRow(
          app: 'cursor',
          capturedAt: 0,
          windowKind: 'billing_period',
          used: 2,
          limitValue: 200,
        ),
      ],
    );

    await tester.pumpWidget(_wrap(group));

    final progress = tester.widget<LinearProgressIndicator>(
      find.byType(LinearProgressIndicator),
    );
    expect(progress.value, closeTo(0.01, 1e-9));
  });
}
