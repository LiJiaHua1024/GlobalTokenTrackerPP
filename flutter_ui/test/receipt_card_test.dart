import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/models.dart';
import 'package:globaltokentracker_ui/widgets/receipt_card.dart';

OverviewData _receiptData({
  List<ShareRow>? byModel,
  List<AppSummary>? byApp,
  Totals? span,
}) {
  final resolvedSpan =
      span ??
      Totals(
        events: 1342,
        inputTokens: 3200000,
        outputTokens: 1100000,
        reasoningTokens: 300000,
        cacheReadTokens: 41900000,
        cacheWriteTokens: 2100000,
        credits: 0,
        costUsd: 31.84,
        activeMs: 3600000,
      );

  return OverviewData(
    today: resolvedSpan,
    span: resolvedSpan,
    all: resolvedSpan,
    rangeType: 'week',
    byApp:
        byApp ??
        <AppSummary>[
          AppSummary(
            app: 'Claude Code',
            events: 900,
            inputTokens: 2000000,
            outputTokens: 800000,
            reasoningTokens: 200000,
            cacheReadTokens: 28000000,
            cacheWriteTokens: 1200000,
            credits: 0,
            costUsd: 21.40,
          ),
          AppSummary(
            app: 'Codex CLI',
            events: 442,
            inputTokens: 1200000,
            outputTokens: 300000,
            reasoningTokens: 100000,
            cacheReadTokens: 13900000,
            cacheWriteTokens: 900000,
            credits: 0,
            costUsd: 10.44,
          ),
        ],
    byModel:
        byModel ??
        <ShareRow>[
          ShareRow(
            name: 'claude-opus-5',
            events: 500,
            tokens: 27900000,
            costUsd: 19.62,
          ),
          ShareRow(
            name: 'gpt-6-astra',
            events: 320,
            tokens: 13900000,
            costUsd: 9.52,
          ),
        ],
    daily: const <TrendBucket>[],
    activity: const <ActivityDay>[],
    apps: const <String>['Claude Code'],
    models: const <String>['claude-opus-5'],
    quotas: const <QuotaRow>[],
    quotaGroups: const <QuotaGroup>[],
    tzOffset: '+08:00',
  );
}

Widget _host(OverviewData data, {String handle = 'alex_engineer'}) {
  return MaterialApp(
    home: Scaffold(
      body: SingleChildScrollView(
        child: ReceiptShareCard(
          data: data,
          periodLabel: '近 7 天统计',
          rangeKey: 'week',
          customHandle: handle,
        ),
      ),
    ),
  );
}

void main() {
  testWidgets('receipt prints header, totals and line items', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    await tester.pumpWidget(_host(_receiptData()));
    await tester.pumpAndSettle();

    expect(find.text('GLOBAL TOKEN TRACKER ++'), findsOneWidget);
    expect(find.text('谢谢惠顾'), findsOneWidget);
    expect(find.textContaining('WEEKLY'), findsOneWidget);
    expect(find.text('alex_engineer'), findsOneWidget);

    // 3.2M + 1.1M + 41.9M = 46.2M total tokens.
    expect(find.text('46.20M'), findsOneWidget);
    expect(find.text('\$31.84'), findsOneWidget);
    expect(find.text('claude-opus-5'), findsOneWidget);
    expect(find.text('27.90M'), findsOneWidget);
    expect(find.text('\$19.62'), findsOneWidget);
  });

  testWidgets('receipt falls back to a local cashier with no handle', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    await tester.pumpWidget(_host(_receiptData(), handle: '   '));
    await tester.pumpAndSettle();

    expect(find.text('LOCAL ONLY'), findsOneWidget);
  });

  testWidgets('receipt stays within the roll with an empty ledger', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final empty = _receiptData(
      byModel: <ShareRow>[],
      byApp: <AppSummary>[],
      span: Totals(
        events: 0,
        inputTokens: 0,
        outputTokens: 0,
        reasoningTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        credits: 0,
        costUsd: 0,
        activeMs: 0,
      ),
    );

    await tester.pumpWidget(_host(empty));
    await tester.pumpAndSettle();

    expect(find.text('0'), findsWidgets);
    expect(find.text('\$0.00'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('receipt truncates long item lists and elides long names', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final many = _receiptData(
      byModel: List<ShareRow>.generate(
        11,
        (i) => ShareRow(
          name: 'provider/model-with-an-extremely-long-identifier-$i',
          events: 10,
          tokens: 1000000 * (11 - i),
          costUsd: 1.5,
        ),
      ),
    );

    await tester.pumpWidget(_host(many));
    await tester.pumpAndSettle();

    // Only six lines fit before the overflow notice.
    expect(find.textContaining('MORE ITEM(S)'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('huge token counts still fit the amount column', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final huge = _receiptData(
      span: Totals(
        events: 98765432,
        inputTokens: 9876543210,
        outputTokens: 1234567890,
        reasoningTokens: 0,
        cacheReadTokens: 98765432100,
        cacheWriteTokens: 0,
        credits: 0,
        costUsd: 1234567.89,
        activeMs: 0,
      ),
    );

    await tester.pumpWidget(_host(huge));
    await tester.pumpAndSettle();

    // 9,876,543,210 + 1,234,567,890 + 98,765,432,100 = 109.88B.
    expect(find.text('109.88B'), findsOneWidget);
    expect(find.text('\$1234567.89'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('printSlip feeds the slip out and locks export meanwhile', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final key = GlobalKey<ReceiptShareCardState>();
    var printingStates = <bool>[];
    Widget app() => MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: ReceiptShareCard(
            key: key,
            data: _receiptData(),
            periodLabel: '近 7 天统计',
            rangeKey: 'week',
            onPrintingChanged: printingStates.add,
          ),
        ),
      ),
    );

    await tester.pumpWidget(app());
    await tester.pumpAndSettle();

    // The slip is laid out at full height from the start; the feed clips it at
    // paint time, so height is not what signals progress.
    final restingHeight = tester.getSize(find.byType(ReceiptPaper)).height;
    expect(find.byKey(const ValueKey<String>('receipt-leading-edge')), findsNothing);

    // `printSlip` must not be awaited directly: the fake clock only advances
    // inside `pump`, so awaiting the animation here would deadlock.
    final printing = key.currentState!.printSlip();
    await tester.pump();
    expect(printingStates, <bool>[true]);

    // Mid-feed the straight cut at the paper's leading edge is showing, and it
    // travels down the slip as the paper feeds out.
    final leading = find.byKey(const ValueKey<String>('receipt-leading-edge'));
    expect(leading, findsOneWidget);
    final startOffset = tester.getTopLeft(leading).dy;

    await tester.pump(kPrintDuration ~/ 2);
    expect(
      tester.getTopLeft(leading).dy,
      greaterThan(startOffset + restingHeight * 0.2),
    );

    // Land past the end rather than exactly on it: ClampingSimulation.isDone
    // is a strict `>`, so a pump of exactly kPrintDuration leaves the ticker
    // one frame short of stopping and the future below never resolves.
    await tester.pump(kPrintDuration);
    await tester.pump(const Duration(milliseconds: 20));
    await printing;

    // Feed done: the cut is gone, replaced by the torn edge, and the lock on
    // image export is released.
    expect(find.byKey(const ValueKey<String>('receipt-leading-edge')), findsNothing);
    expect(tester.getSize(find.byType(ReceiptPaper)).height, restingHeight);
    expect(printingStates, <bool>[true, false]);
  });

  testWidgets('autoPrint feeds the slip without a button press', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    var printingStates = <bool>[];
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: ReceiptShareCard(
              data: _receiptData(),
              periodLabel: '近 7 天统计',
              autoPrint: true,
              onPrintingChanged: printingStates.add,
            ),
          ),
        ),
      ),
    );

    // autoPrint is consumed after the height lands, so a frame or two are
    // needed before the feed begins.
    await tester.pump();
    await tester.pump();
    expect(printingStates, <bool>[true]);

    await tester.pump(kPrintDuration);
    await tester.pump(const Duration(milliseconds: 20));
    expect(printingStates, <bool>[true, false]);
  });

  testWidgets('autoPrint stays quiet until the host raises it', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    var printingStates = <bool>[];
    Widget app({required bool autoPrint}) => MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: ReceiptShareCard(
            data: _receiptData(),
            periodLabel: '近 7 天统计',
            autoPrint: autoPrint,
            onPrintingChanged: printingStates.add,
          ),
        ),
      ),
    );

    await tester.pumpWidget(app(autoPrint: false));
    await tester.pumpAndSettle();
    expect(printingStates, isEmpty);

    // Switching to the receipt style raises the flag, which replays the feed.
    await tester.pumpWidget(app(autoPrint: true));
    await tester.pump();
    await tester.pump();
    expect(printingStates, <bool>[true]);
  });

  test('feed curve varies speed without ever halting', () {
    expect(kFeedCurve.transform(0), 0);
    expect(kFeedCurve.transform(1), 1);

    // Monotonic — the head never runs the paper backwards.
    var previous = -1.0;
    for (var i = 0; i <= 1000; i++) {
      final value = kFeedCurve.transform(i / 1000);
      expect(value, greaterThanOrEqualTo(previous));
      previous = value;
    }

    // The velocity profile must stay strictly positive through the middle of
    // the feed. A zero-velocity plateau is the "sprint, freeze, sprint" shape
    // that reads as mechanical.
    for (var i = 200; i <= 800; i++) {
      expect(
        receiptFeedVelocity(i / 1000),
        greaterThan(0.0),
        reason: 'feed must never fully stop at t=${i / 1000}',
      );
    }

    // …while still swinging hard enough to read as a machine: some moments
    // much slower than others.
    final speeds = List<double>.generate(
      401,
      (i) => receiptFeedVelocity(0.1 + i / 1000 * 0.8),
    );
    final slowest = speeds.reduce(math.min);
    final fastest = speeds.reduce(math.max);
    expect(fastest / slowest, greaterThan(2.0));
  });

  testWidgets('printer chrome renders outside the slip', (
    WidgetTester tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1300, 820));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: Column(
              children: <Widget>[
                const ReceiptPrinter(),
                ReceiptShareCard(
                  data: _receiptData(),
                  periodLabel: '近 7 天统计',
                ),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byType(ReceiptPrinter), findsOneWidget);
    expect(find.byType(ReceiptPaper), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}