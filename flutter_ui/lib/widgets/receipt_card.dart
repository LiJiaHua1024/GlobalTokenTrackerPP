import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../core/models.dart';

/// Width of the printed slip, in logical pixels. Narrower than the gradient
/// share cards on purpose — thermal paper comes off a 58mm roll.
const double kReceiptWidth = 420;

/// How long a full feed takes.
const Duration kPrintDuration = Duration(milliseconds: 2800);

/// Thermal-paper palette. Deliberately not theme-derived: the effect lands
/// precisely because the slip stays the same cool near-white in both modes.
abstract final class _Ink {
  // Real thermal stock is a bleached, faintly cool white — the yellow-cream
  // of cheap foidler paper reads as "old receipt", not "just printed".
  static const paper = Color(0xFFFCFDFD);
  static const paperEdge = Color(0xFFB9C0C3);
  // A print head that has just fired burns near-solid black. Nothing here is
  // a low-contrast grey; the tonal steps are small and only separate a field
  // label from its value.
  static const black = Color(0xFF0C0D0E);
  static const soft = Color(0xFF2C3134);
  static const muted = Color(0xFF43494D);
  static const rule = Color(0xFF6B7377);

  static const printerBody = Color(0xFF2B2D33);
  static const printerHighlight = Color(0xFF474A53);
  static const printerSlot = Color(0xFF0C0D10);
}

/// Latin falls to Doto. Chinese is absent from that face, so it drops to the
/// system CJK font — which is exactly how real thermal printers behave when a
/// CJK ROM font is loaded alongside the built-in ASCII one.
const List<String> _receiptFontFallback = <String>[
  'Microsoft YaHei',
  'SimSun',
];

/// Builds a thermal-receipt style. Weight is driven purely by the variable
/// axis: pairing it with a matching `fontWeight` thins the glyphs a second
/// time, so the two must never be set together.
TextStyle _mono({
  double size = 11,
  FontWeight? weight = FontWeight.w600,
  Color color = _Ink.black,
  double letterSpacing = 0,
}) {
  return TextStyle(
    fontFamily: 'Doto',
    fontFamilyFallback: _receiptFontFallback,
    fontSize: size,
    color: color,
    letterSpacing: letterSpacing,
    height: 1.3,
    fontVariations: weight == null
        ? null
        : <FontVariation>[FontVariation('wght', weight.value.toDouble())],
  );
}

/// A supermarket-receipt rendering of the usage stats, feeding out of a little
/// thermal printer body.
///
/// The feed is deliberately not a smooth wipe: [kFeedCurve] varies the paper
/// speed over the course of the print and the slip shivers as the head hunts.
class ReceiptShareCard extends StatefulWidget {
  final OverviewData data;
  final String periodLabel;
  final String? rangeKey;
  final String? customHandle;
  final bool showCost;
  final bool showModels;
  final bool showTools;
  final bool showCache;

  /// Fires when the feed starts and when it finishes, so the host can lock out
  /// image export while the slip is only half printed.
  final ValueChanged<bool>? onPrintingChanged;

  /// Feed the slip automatically once it has been measured. Hosts raise this
  /// when the user switches to the receipt style; it is consumed on use, so
  /// leaving it set does not re-print on every unrelated rebuild.
  final bool autoPrint;

  const ReceiptShareCard({
    super.key,
    required this.data,
    required this.periodLabel,
    this.rangeKey,
    this.customHandle,
    this.showCost = true,
    this.showModels = true,
    this.showTools = true,
    this.showCache = true,
    this.onPrintingChanged,
    this.autoPrint = false,
  });

  @override
  State<ReceiptShareCard> createState() => ReceiptShareCardState();
}

class ReceiptShareCardState extends State<ReceiptShareCard>
    with SingleTickerProviderStateMixin {
  final GlobalKey _paperKey = GlobalKey();

  late final AnimationController _feed = AnimationController(
    vsync: this,
    duration: kPrintDuration,
  );

  /// True once the slip has finished at least one feed. Guards the automatic
  /// print so the very first layout is not torn through from empty.
  bool _hasPrinted = false;

  /// Set when this widget is mounted with [autoPrint], cleared as soon as it
  /// has been consumed.
  bool _autoPrintPending = false;

  /// Natural height of the slip, measured on the first frame when it is laid
  /// out fully. Holding this keeps the surrounding layout from rescaling as the
  /// paper grows.
  double? _paperHeight;

  /// Replays the feed.
  Future<void> printSlip() async {
    _hasPrinted = true;
    widget.onPrintingChanged?.call(true);
    try {
      await _feed.forward(from: 0);
    } finally {
      widget.onPrintingChanged?.call(false);
    }
  }

  @override
  void initState() {
    super.initState();
    // Start fully printed so the first layout can measure the slip, then roll
    // back and feed once the height is known.
    _feed.value = 1;
    _autoPrintPending = widget.autoPrint;
    _scheduleMeasure();
  }

  @override
  void didUpdateWidget(ReceiptShareCard oldWidget) {
    super.didUpdateWidget(oldWidget);
    // A host that re-raises autoPrint (switching to this style, say) gets the
    // feed replayed.
    if (widget.autoPrint && !oldWidget.autoPrint) {
      _autoPrintPending = true;
    }
    // Different data means a different number of rows. Drop the cached height
    // as well, otherwise the taller slip is laid out one frame under the old
    // fixed height and overflows before the re-measure lands.
    if (oldWidget.data != widget.data ||
        oldWidget.periodLabel != widget.periodLabel ||
        oldWidget.showModels != widget.showModels ||
        oldWidget.showCache != widget.showCache ||
        oldWidget.showCost != widget.showCost ||
        oldWidget.showTools != widget.showTools) {
      _paperHeight = null;
      _scheduleMeasure();
    }
  }

  @override
  void dispose() {
    _feed.dispose();
    super.dispose();
  }

  void _scheduleMeasure() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _measurePaper();
      _maybeAutoPrint();
    });
  }

  void _measurePaper() {
    if (!mounted) return;
    final box = _paperKey.currentContext?.findRenderObject() as RenderBox?;
    if (box == null || !box.hasSize) return;
    final height = box.size.height;
    if (height <= 0 || height == _paperHeight) return;
    setState(() => _paperHeight = height);
  }

  /// Waits until the slip has a measured height, otherwise the first frames of
  /// the feed would play out with nothing pinned to clip against.
  void _maybeAutoPrint() {
    if (!_autoPrintPending || _paperHeight == null) return;
    _autoPrintPending = false;
    printSlip();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _feed,
      builder: (context, _) {
        final raw = _feed.value.clamp(0.0, 1.0);
        final t = kFeedCurve.transform(raw);
        final height = _paperHeight;

        // Before the first measurement just draw the slip whole; the very
        // next frame pins the height and the feed can start.
        if (height == null || height <= 0) {
          return _paper(t: 1);
        }

        return SizedBox(
          height: height,
          child: Stack(
            clipBehavior: Clip.none,
            children: <Widget>[
              Positioned.fill(
                child: ClipRect(
                  clipper: _FeedClipper(t),
                  // The head drags the paper sideways as it strikes each line.
                  child: Transform.translate(
                    offset: _headShiver(raw),
                    child: _paper(t: 1),
                  ),
                ),
              ),
              // The hot leading edge of paper still inside the printer.
              if (t < 0.999)
                Positioned(
                  top: 0,
                  left: 0,
                  right: 0,
                  child: Transform.translate(
                    offset: Offset(0, t * height),
                    child: const _PaperLeadingEdge(),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }

  /// Side-to-side hunt of the print head. Its amplitude tracks feed speed, so
  /// the slip buzzes while running and goes quiet during a hesitation — the
  /// opposite of the old spike-per-step version, which jolted hardest exactly
  /// when the paper had stopped.
  Offset _headShiver(double t) {
    if (t <= 0.0 || t >= 1.0) return Offset.zero;
    final amplitude = (receiptFeedVelocity(t) * 0.4).clamp(0.0, 0.55);
    return Offset(
      math.sin(t * 118 + 0.4) * amplitude + math.sin(t * 47) * amplitude * 0.35,
      0,
    );
  }

  Widget _paper({required double t}) {
    return KeyedSubtree(
      key: _paperKey,
      child: ReceiptPaper(
        data: widget.data,
        periodLabel: widget.periodLabel,
        rangeKey: widget.rangeKey,
        customHandle: widget.customHandle,
        showCost: widget.showCost,
        showModels: widget.showModels,
        showTools: widget.showTools,
        showCache: widget.showCache,
        torn: t >= 0.999,
      ),
    );
  }
}

/// Reveals the top `t` fraction of the slip; the rest stays clipped inside
/// the printer.
class _FeedClipper extends CustomClipper<Rect> {
  final double t;

  const _FeedClipper(this.t);

  @override
  Rect getClip(Size size) => Rect.fromLTWH(0, 0, size.width, size.height * t);

  @override
  bool shouldReclip(_FeedClipper oldClipper) => oldClipper.t != t;
}

/// Maps wall-clock time onto how much paper has come out.
///
/// A print head does not stop and restart: it feeds at a broadly steady speed,
/// judders, and only occasionally hesitates for a beat. Quantising the motion
/// into "sprint, freeze, sprint" reads as mechanical, so the motion is instead
/// built from a *velocity* profile and integrated — the paper never fully
/// halts, and every hesitation is a different length and depth.
final ReceiptFeedCurve kFeedCurve = ReceiptFeedCurve();

/// Hesitations as (centre, width, depth). Unevenly spaced and deliberately
/// unequal: a repeating rhythm would read as a loop.
const List<({double at, double width, double depth})> _hesitations =
    <({double at, double width, double depth})>[
      (at: 0.13, width: 0.030, depth: 0.52),
      (at: 0.29, width: 0.021, depth: 0.38),
      (at: 0.44, width: 0.046, depth: 0.70),
      (at: 0.61, width: 0.025, depth: 0.46),
      (at: 0.76, width: 0.033, depth: 0.60),
      (at: 0.90, width: 0.019, depth: 0.42),
    ];

/// Feed speed at [t], in arbitrary units where 1.0 is the nominal rate.
double receiptFeedVelocity(double t) {
  var v = 1.0;

  // Print-head judder. Two incommensurate frequencies so the wobble never
  // settles into a visible metronome.
  v += 0.12 * math.sin(t * 61) + 0.07 * math.sin(t * 143 + 1.1);

  // Brief hesitations, each a slowdown rather than a dead stop.
  for (final h in _hesitations) {
    final x = (t - h.at) / h.width;
    v -= h.depth * math.exp(-x * x * 2.2);
  }

  // The head grabs the paper at the start and settles at the end.
  if (t < 0.05) v *= math.sqrt(t / 0.05);
  if (t > 0.95) v *= math.sqrt((1 - t) / 0.05);

  return math.max(v, 0.06);
}

class ReceiptFeedCurve extends Curve {
  ReceiptFeedCurve() : _cumulative = _buildCumulative();

  final List<double> _cumulative;

  static const int _samples = 1024;

  static List<double> _buildCumulative() {
    final cumulative = List<double>.filled(_samples + 1, 0);
    for (var i = 1; i <= _samples; i++) {
      final a = receiptFeedVelocity((i - 1) / _samples);
      final b = receiptFeedVelocity(i / _samples);
      cumulative[i] = cumulative[i - 1] + (a + b) * 0.5 / _samples;
    }
    // Normalise so the slip always finishes printing, whatever the profile.
    final total = cumulative[_samples];
    for (var i = 0; i <= _samples; i++) {
      cumulative[i] /= total;
    }
    return cumulative;
  }

  @override
  double transformInternal(double t) {
    final x = t.clamp(0.0, 1.0) * _samples;
    final i = x.floor();
    if (i >= _samples) return 1.0;
    final f = x - i;
    return _cumulative[i] + (_cumulative[i + 1] - _cumulative[i]) * f;
  }
}

/// The slip itself: thermal paper, monospace ink, torn off the roll once the
/// feed finishes. Also used standalone, so it is public.
class ReceiptPaper extends StatelessWidget {
  final OverviewData data;
  final String periodLabel;
  final String? rangeKey;
  final String? customHandle;
  final bool showCost;
  final bool showModels;
  final bool showTools;
  final bool showCache;
  final bool torn;

  const ReceiptPaper({
    super.key,
    required this.data,
    required this.periodLabel,
    this.rangeKey,
    this.customHandle,
    this.showCost = true,
    this.showModels = true,
    this.showTools = true,
    this.showCache = true,
    this.torn = true,
  });

  String get _effectiveRangeKey {
    if (rangeKey != null && rangeKey!.isNotEmpty) return rangeKey!;
    if (data.rangeType.isNotEmpty) return data.rangeType;
    return 'week';
  }

  String get _periodCode {
    switch (_effectiveRangeKey) {
      case 'today':
        return 'DAILY';
      case 'week':
        return 'WEEKLY';
      case 'month':
        return 'MONTHLY';
      case 'all':
        return 'ALL-TIME';
      default:
        return 'PERIOD';
    }
  }

  @override
  Widget build(BuildContext context) {
    final span = data.span;

    final sortedModels = List<ShareRow>.from(data.byModel)
      ..sort((a, b) => b.tokens.compareTo(a.tokens));
    final sortedTools = List<AppSummary>.from(data.byApp)
      ..sort((a, b) => b.totalTokens.compareTo(a.totalTokens));

    final cacheHitRate = span.totalTokens > 0
        ? (span.cacheReadTokens / span.totalTokens * 100).clamp(0.0, 100.0)
        : 0.0;
    final perMillion = span.totalTokens > 0
        ? span.costUsd / (span.totalTokens / 1000000)
        : 0.0;
    final topTool = sortedTools.isEmpty ? null : sortedTools.first;
    final topToolShare = span.totalTokens > 0 && topTool != null
        ? topTool.totalTokens / span.totalTokens * 100
        : 0.0;

    final now = DateTime.now();
    final printedAt = DateFormat('yyyy-MM-dd HH:mm').format(now);
    final orderNo = '#${DateFormat('yyyyMMdd-HHmm').format(now)}';
    final handle = customHandle?.trim() ?? '';
    final cashier = handle.isEmpty ? 'LOCAL ONLY' : handle;

    final modelLines =
        showModels ? sortedModels.take(6).toList() : <ShareRow>[];
    final hiddenModels = showModels && sortedModels.length > modelLines.length
        ? sortedModels.length - modelLines.length
        : 0;

    // Hold the roll width even when a parent hands down a wider box: the
    // line-item rows use fixed gutters and would overflow at any other width.
    return Center(
      child: SizedBox(
        width: kReceiptWidth,
        child: _PaperGrain(
          child: Container(
            decoration: const BoxDecoration(
              color: _Ink.paper,
              boxShadow: <BoxShadow>[
                BoxShadow(
                  color: Color(0x1F000000),
                  blurRadius: 16,
                  offset: Offset(0, 7),
                ),
              ],
            ),
            child: ClipPath(
              clipper: _TornEdgeClipper(torn: torn),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(26, 20, 26, 24),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    _header(),
                    const SizedBox(height: 12),
                    const _Rule(),
                    const SizedBox(height: 10),

                    _MetaRow(label: 'TIME', value: printedAt),
                    _MetaRow(
                        label: 'PERIOD', value: '$periodLabel ($_periodCode)'),
                    _MetaRow(label: 'CASHIER', value: cashier),
                    _MetaRow(label: 'ORDER', value: orderNo),
                    const SizedBox(height: 10),
                    const _Rule(),

                    if (showModels) ...<Widget>[
                      const SizedBox(height: 10),
                      _columnHeader(),
                      const SizedBox(height: 7),
                      for (final m in modelLines)
                        _modelRow(m, span.totalTokens),
                      if (hiddenModels > 0)
                        Padding(
                          padding: const EdgeInsets.only(top: 3),
                          child: Text(
                            '... AND $hiddenModels MORE ITEM(S)',
                            style: _mono(size: 9, color: _Ink.muted),
                          ),
                        ),
                      const SizedBox(height: 10),
                      const _Rule(),
                    ],

                    // Totals — the visual centre of gravity of any receipt.
                    const SizedBox(height: 10),
                    _totalRow(
                      'TOTAL TOKENS',
                      _compact(span.totalTokens),
                      size: 13,
                      weight: FontWeight.w700,
                    ),
                    if (showCost)
                      _totalRow(
                        'TOTAL COST',
                        '\$${span.costUsd.toStringAsFixed(2)}',
                        size: 19,
                        weight: FontWeight.w800,
                      ),
                    if (showCost)
                      _totalRow(
                        'UNIT PRICE',
                        '\$${perMillion.toStringAsFixed(4)} / 1M',
                        size: 10,
                        color: _Ink.soft,
                      ),
                    const SizedBox(height: 10),
                    const _Rule(heavy: true),
                    const SizedBox(height: 10),

                    _MetaRow(label: 'INPUT', value: _compact(span.inputTokens)),
                    _MetaRow(
                        label: 'OUTPUT', value: _compact(span.outputTokens)),
                    _MetaRow(
                      label: 'CACHE READ',
                      value: _compact(span.cacheReadTokens),
                    ),
                    if (span.reasoningTokens > 0)
                      _MetaRow(
                        label: 'REASONING',
                        value: _compact(span.reasoningTokens),
                      ),
                    const SizedBox(height: 10),
                    const _Rule(),
                    const SizedBox(height: 10),

                    _MetaRow(
                      label: 'REQUESTS',
                      value: NumberFormat.decimalPattern().format(span.events),
                    ),
                    if (showCache)
                      _MetaRow(
                        label: 'CACHE HIT',
                        value: '${cacheHitRate.toStringAsFixed(1)}%',
                      ),
                    if (showTools && topTool != null)
                      _MetaRow(
                        label: 'TOP TOOL',
                        value:
                            '${topTool.app} ${topToolShare.toStringAsFixed(0)}%',
                      ),
                    const SizedBox(height: 12),
                    const _Rule(),
                    const SizedBox(height: 14),

                    _footer(orderNo, span),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Widget _header() {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
          decoration:
              BoxDecoration(border: Border.all(color: _Ink.black, width: 2)),
          child: Text(
            'GLOBAL TOKEN TRACKER ++',
            style: _mono(size: 13, weight: FontWeight.w800, letterSpacing: 1.4),
          ),
        ),
        const SizedBox(height: 7),
        Text(
          'AI USAGE SETTLEMENT CENTRE',
          style: _mono(size: 9, color: _Ink.soft, letterSpacing: 1.1),
        ),
        const SizedBox(height: 3),
        Text('AI 用量结算中心', style: _mono(size: 10, color: _Ink.soft)),
      ],
    );
  }

  Widget _columnHeader() {
    return Row(
      children: <Widget>[
        Expanded(
          child: Text(
            'ITEM',
            style: _mono(size: 9, color: _Ink.muted, letterSpacing: 1.1),
          ),
        ),
        const SizedBox(width: 8),
        SizedBox(
          width: 70,
          child: Text(
            'QTY',
            textAlign: TextAlign.right,
            style: _mono(size: 9, color: _Ink.muted, letterSpacing: 1.1),
          ),
        ),
        const SizedBox(width: 12),
        SizedBox(
          width: 76,
          child: Text(
            'AMOUNT',
            textAlign: TextAlign.right,
            style: _mono(size: 9, color: _Ink.muted, letterSpacing: 1.1),
          ),
        ),
      ],
    );
  }

  Widget _modelRow(ShareRow m, int spanTokens) {
    final share = spanTokens > 0 ? m.tokens / spanTokens : 0.0;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Expanded(
                child: Text(
                  m.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: _mono(size: 11),
                ),
              ),
              const SizedBox(width: 8),
              SizedBox(
                width: 70,
                child: Text(
                  _compact(m.tokens),
                  textAlign: TextAlign.right,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: _mono(size: 11),
                ),
              ),
              const SizedBox(width: 12),
              SizedBox(
                width: 76,
                child: Text(
                  '\$${m.costUsd.toStringAsFixed(2)}',
                  textAlign: TextAlign.right,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: _mono(size: 11, weight: FontWeight.w700),
                ),
              ),
            ],
          ),
          const SizedBox(height: 4),
          // Share of spend, drawn as rows of thermal dashes.
          _DashMeter(fraction: share),
        ],
      ),
    );
  }

  Widget _totalRow(
    String label,
    String value, {
    double size = 12,
    FontWeight? weight,
    Color color = _Ink.black,
  }) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 5),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: <Widget>[
          // Label and value stay flexible so an outsized amount ellipsizes
          // instead of pushing the row past the paper edge.
          Flexible(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: _mono(size: size, weight: weight, color: color),
            ),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.only(bottom: 3),
              child: CustomPaint(
                painter: _LeaderDotsPainter(),
                size: const Size(double.infinity, 8),
              ),
            ),
          ),
          const SizedBox(width: 10),
          Flexible(
            child: Text(
              value,
              textAlign: TextAlign.right,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: _mono(size: size, weight: weight, color: color),
            ),
          ),
        ],
      ),
    );
  }

  Widget _footer(String orderNo, Totals span) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: <Widget>[
        Text(
          '谢谢惠顾',
          style: _mono(size: 15, weight: FontWeight.w700, letterSpacing: 3),
        ),
        const SizedBox(height: 4),
        Text(
          'THANK YOU  ·  NOTHING LEAVES THIS MACHINE',
          textAlign: TextAlign.center,
          style: _mono(size: 8, color: _Ink.soft, letterSpacing: 0.7),
        ),
        const SizedBox(height: 12),
        Center(
          child: CustomPaint(
            painter: _BarcodePainter(
              seed: '$orderNo|${span.totalTokens}|${span.costUsd}',
            ),
            size: const Size(220, 44),
          ),
        ),
        const SizedBox(height: 7),
        Text(orderNo.substring(1), style: _mono(size: 10, letterSpacing: 3)),
        const SizedBox(height: 2),
        Text(
          'GLOBALTOKENTRACKER  ·  LOCAL LEDGER',
          style: _mono(size: 8, color: _Ink.muted, letterSpacing: 0.5),
        ),
      ],
    );
  }

  /// Same K/M/B/T shorthand the rest of the app uses, kept local so the slip
  /// does not need a provider read inside every row.
  static String _compact(num v) {
    final abs = v.abs();
    if (abs >= 1000000000000) {
      return '${(v / 1000000000000).toStringAsFixed(2)}T';
    }
    if (abs >= 1000000000) return '${(v / 1000000000).toStringAsFixed(2)}B';
    if (abs >= 1000000) return '${(v / 1000000).toStringAsFixed(2)}M';
    if (abs >= 1000) return '${(v / 1000).toStringAsFixed(1)}K';
    return NumberFormat.decimalPattern().format(v);
  }
}

/// The slot the slip feeds out of. Purely decorative — the host shows this
/// above the capture boundary so exported images contain the paper alone.
class ReceiptPrinter extends StatelessWidget {
  const ReceiptPrinter({super.key});

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: kReceiptWidth + 72,
      height: 52,
      child: Stack(
        clipBehavior: Clip.none,
        alignment: Alignment.topCenter,
        children: <Widget>[
          Container(
            width: kReceiptWidth + 72,
            height: 38,
            decoration: BoxDecoration(
              borderRadius:
                  const BorderRadius.vertical(top: Radius.circular(14)),
              gradient: const LinearGradient(
                begin: Alignment.topCenter,
                end: Alignment.bottomCenter,
                colors: <Color>[_Ink.printerHighlight, _Ink.printerBody],
              ),
              border: Border.all(color: const Color(0xFF1A1C20)),
              boxShadow: const <BoxShadow>[
                BoxShadow(
                    color: Color(0x33000000),
                    blurRadius: 14,
                    offset: Offset(0, 5)),
              ],
            ),
          ),
          // Power LED.
          Positioned(
            top: 13,
            left: 24,
            child: Container(
              width: 7,
              height: 7,
              decoration: const BoxDecoration(
                shape: BoxShape.circle,
                color: Color(0xFF64D27A),
                boxShadow: <BoxShadow>[
                  BoxShadow(color: Color(0x8864D27A), blurRadius: 7),
                ],
              ),
            ),
          ),
          // The throat the paper drops behind.
          Positioned(
            top: 33,
            child: Container(
              width: kReceiptWidth + 16,
              height: 9,
              decoration: BoxDecoration(
                color: _Ink.printerSlot,
                borderRadius: BorderRadius.circular(3),
                boxShadow: const <BoxShadow>[
                  BoxShadow(
                      color: Color(0x66000000),
                      blurRadius: 5,
                      offset: Offset(0, 2)),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Straight, faintly shadowed cut at the bottom of a slip still feeding.
class _PaperLeadingEdge extends StatelessWidget {
  const _PaperLeadingEdge();

  @override
  Widget build(BuildContext context) {
    return Container(
      key: const ValueKey<String>('receipt-leading-edge'),
      width: kReceiptWidth,
      height: 3,
      decoration: BoxDecoration(
        color: _Ink.paper,
        border: Border(bottom: BorderSide(color: _Ink.paperEdge)),
        boxShadow: const <BoxShadow>[
          BoxShadow(
              color: Color(0x16000000), blurRadius: 6, offset: Offset(0, 3)),
        ],
      ),
    );
  }
}

class _MetaRow extends StatelessWidget {
  final String label;
  final String value;

  const _MetaRow({required this.label, required this.value});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 3),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          SizedBox(
            width: 106,
            child: Text(
              label,
              style: _mono(size: 9.5, color: _Ink.muted, letterSpacing: 0.7),
            ),
          ),
          Expanded(
            child: Text(
              value,
              textAlign: TextAlign.right,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: _mono(size: 10.5),
            ),
          ),
        ],
      ),
    );
  }
}

class _Rule extends StatelessWidget {
  final bool heavy;

  const _Rule({this.heavy = false});

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: heavy ? 5 : 2,
      width: double.infinity,
      child: CustomPaint(painter: _RulePainter(heavy: heavy)),
    );
  }
}

class _RulePainter extends CustomPainter {
  final bool heavy;

  const _RulePainter({this.heavy = false});

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = _Ink.rule
      ..strokeWidth = heavy ? 1.3 : 1
      ..strokeCap = StrokeCap.round;
    final mid = size.height / 2;
    if (heavy) {
      canvas.drawLine(
          Offset(0, mid - 1.6), Offset(size.width, mid - 1.6), paint);
      canvas.drawLine(
          Offset(0, mid + 1.6), Offset(size.width, mid + 1.6), paint);
      return;
    }
    // Dashed, the way a thermal head actually prints a separator.
    const dash = 5.0;
    const gap = 4.0;
    for (var x = 0.0; x < size.width; x += dash + gap) {
      canvas.drawLine(
        Offset(x, mid),
        Offset(math.min(x + dash, size.width), mid),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(_RulePainter oldDelegate) => oldDelegate.heavy != heavy;
}

class _LeaderDotsPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()..color = _Ink.rule;
    const step = 5.0;
    final mid = size.height / 2;
    for (var x = 0.0; x < size.width; x += step) {
      canvas.drawCircle(Offset(x, mid), 0.9, paint);
    }
  }

  @override
  bool shouldRepaint(_LeaderDotsPainter oldDelegate) => false;
}

/// Barcode derived from the transaction, so a given receipt always encodes to
/// the same bars while different data prints differently.
class _BarcodePainter extends CustomPainter {
  final String seed;

  const _BarcodePainter({required this.seed});

  @override
  void paint(Canvas canvas, Size size) {
    var hash = 0x811C9DC5;
    for (final unit in seed.codeUnits) {
      hash ^= unit;
      hash = (hash * 0x01000193) & 0xFFFFFFFF;
    }

    final paint = Paint()..color = _Ink.black;
    const barCount = 44;
    final unit = size.width / (barCount * 2 + 1);
    var x = 0.0;
    for (var i = 0; i < barCount; i++) {
      // Re-hash per bar so widths do not fall into a visible pattern.
      hash ^= (i * 0x9E3779B1) & 0xFFFFFFFF;
      hash = (hash * 0x01000193) & 0xFFFFFFFF;
      final w = (hash & 0x1) == 1 ? unit * 2 : unit;
      canvas.drawRect(Rect.fromLTWH(x, 0, w, size.height), paint);
      x += w + unit;
    }
  }

  @override
  bool shouldRepaint(_BarcodePainter oldDelegate) => oldDelegate.seed != seed;
}

/// Share-of-spend meter rendered as rows of thermal dashes.
class _DashMeter extends StatelessWidget {
  final double fraction;

  const _DashMeter({required this.fraction});

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 4,
      width: double.infinity,
      child: LayoutBuilder(
        builder: (context, constraints) {
          final filled = constraints.maxWidth * fraction.clamp(0.0, 1.0);
          return Stack(
            children: <Widget>[
              CustomPaint(
                painter: _DashRowPainter(color: _Ink.paperEdge),
                size: Size(constraints.maxWidth, 4),
              ),
              if (filled > 0)
                CustomPaint(
                  painter: _DashRowPainter(color: _Ink.black),
                  size: Size(filled, 4),
                ),
            ],
          );
        },
      ),
    );
  }
}

class _DashRowPainter extends CustomPainter {
  final Color color;

  const _DashRowPainter({required this.color});

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()..color = color;
    for (var x = 0.0; x < size.width; x += 4) {
      canvas.drawRect(Rect.fromLTWH(x, 0, 2, size.height), paint);
    }
  }

  @override
  bool shouldRepaint(_DashRowPainter oldDelegate) => oldDelegate.color != color;
}

/// Fibrous speckle over the whole slip. Offsets are generated once and the
/// painter is cached, so this costs one draw call and never recomputes.
class _PaperGrain extends StatefulWidget {
  final Widget child;

  const _PaperGrain({required this.child});

  @override
  State<_PaperGrain> createState() => _PaperGrainState();
}

class _PaperGrainState extends State<_PaperGrain> {
  late final List<Offset> _specks = _buildSpecks();

  static List<Offset> _buildSpecks() {
    final rng = math.Random(0x5EED);
    return List<Offset>.generate(
      340,
      (_) => Offset(rng.nextDouble(), rng.nextDouble()),
    );
  }

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      // Foreground so the grain sits on top of the ink, not behind the paper.
      foregroundPainter: _GrainPainter(_specks),
      child: widget.child,
    );
  }
}

class _GrainPainter extends CustomPainter {
  final List<Offset> specks;

  _GrainPainter(this.specks);

  // The speck positions are unit-square; scale them once per size rather than
  // reallocating on every repaint during the feed.
  Size? _cachedSize;
  late List<Offset> _scaled = const <Offset>[];

  @override
  void paint(Canvas canvas, Size size) {
    if (_cachedSize != size) {
      _scaled = <Offset>[
        for (final s in specks) Offset(s.dx * size.width, s.dy * size.height),
      ];
      _cachedSize = size;
    }
    canvas.drawPoints(
      ui.PointMode.points,
      _scaled,
      Paint()
        ..color = const Color(0x0F1A1A1A)
        ..strokeWidth = 1
        ..strokeCap = StrokeCap.round,
    );
  }

  @override
  bool shouldRepaint(_GrainPainter oldDelegate) => false;
}

/// Squares off the slip while it feeds, and gives it a torn-off edge once the
/// feed completes.
class _TornEdgeClipper extends CustomClipper<Path> {
  final bool torn;

  const _TornEdgeClipper({required this.torn});

  @override
  Path getClip(Size size) {
    if (!torn || size.height <= 0 || size.width <= 0) {
      return Path()..addRect(Offset.zero & size);
    }

    // Real tears are not a perfect zigzag, so jitter each tooth deterministically.
    const teeth = 18;
    final toothWidth = size.width / teeth;
    final rng = math.Random(0x7EA5);

    final path = Path()..moveTo(0, 0);
    path.lineTo(size.width, 0);
    path.lineTo(size.width, size.height);
    for (var i = teeth; i >= 1; i--) {
      final center = i * toothWidth - toothWidth / 2;
      final depth = toothWidth * (0.22 + rng.nextDouble() * 0.16);
      path.lineTo(center, size.height - depth);
      path.lineTo(center - toothWidth / 2, size.height);
    }
    path.close();
    return path;
  }

  @override
  bool shouldReclip(_TornEdgeClipper oldClipper) => oldClipper.torn != torn;
}
