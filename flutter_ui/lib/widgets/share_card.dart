import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' as ui;
import 'package:file_picker/file_picker.dart';
import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';
import 'package:pasteboard/pasteboard.dart';
import 'package:provider/provider.dart';

import '../core/ffi_bridge.dart';
import '../core/models.dart';
import '../core/theme.dart';

/// Preset visual themes for the share card
enum ShareCardStyle {
  m3Dynamic,
  midnightDark,
  auroraGlow,
  crispMinimal,
}

extension ShareCardStyleExtension on ShareCardStyle {
  String get label {
    switch (this) {
      case ShareCardStyle.m3Dynamic:
        return 'MD3 动态';
      case ShareCardStyle.midnightDark:
        return '黑曜夜空';
      case ShareCardStyle.auroraGlow:
        return '极光幻境';
      case ShareCardStyle.crispMinimal:
        return '现代极简';
    }
  }

  IconData get icon {
    switch (this) {
      case ShareCardStyle.m3Dynamic:
        return Icons.palette_outlined;
      case ShareCardStyle.midnightDark:
        return Icons.dark_mode_outlined;
      case ShareCardStyle.auroraGlow:
        return Icons.auto_awesome;
      case ShareCardStyle.crispMinimal:
        return Icons.light_mode_outlined;
    }
  }
}

/// Trend chart rendering mode in the share card
enum ShareTrendChartType {
  multiModel,
  classicBars,
}

extension ShareTrendChartTypeExtension on ShareTrendChartType {
  String get label {
    switch (this) {
      case ShareTrendChartType.multiModel:
        return '多模型平滑曲线';
      case ShareTrendChartType.classicBars:
        return '经典走势柱状';
    }
  }

  IconData get icon {
    switch (this) {
      case ShareTrendChartType.multiModel:
        return Icons.insights_rounded;
      case ShareTrendChartType.classicBars:
        return Icons.bar_chart_rounded;
    }
  }
}

/// Resolved palette for rendering the card with sophisticated gradients and atmospheric depth
class ShareCardPalette {
  final BoxDecoration backgroundDecoration;
  final Gradient cardGradient;
  final Color cardBorder;
  final Gradient heroCardGradient;
  final Color heroCardBorder;
  final Gradient heroNumberGradient;
  final Gradient trendBarGradient;
  final Color primary;
  final Color secondary;
  final Color accent;
  final Color textPrimary;
  final Color textSecondary;
  final Color textMuted;
  final Color pillBackground;
  final Color pillBorder;
  final Color glowColor;
  final bool isDark;

  const ShareCardPalette({
    required this.backgroundDecoration,
    required this.cardGradient,
    required this.cardBorder,
    required this.heroCardGradient,
    required this.heroCardBorder,
    required this.heroNumberGradient,
    required this.trendBarGradient,
    required this.primary,
    required this.secondary,
    required this.accent,
    required this.textPrimary,
    required this.textSecondary,
    required this.textMuted,
    required this.pillBackground,
    required this.pillBorder,
    required this.glowColor,
    required this.isDark,
  });

  static ShareCardPalette resolve(ShareCardStyle style, ThemeData appTheme) {
    switch (style) {
      case ShareCardStyle.m3Dynamic:
        final cs = appTheme.colorScheme;
        final isDark = appTheme.brightness == Brightness.dark;
        return ShareCardPalette(
          backgroundDecoration: BoxDecoration(
            gradient: LinearGradient(
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
              stops: const [0.0, 0.45, 1.0],
              colors: isDark
                  ? [
                      cs.surfaceContainerLowest,
                      Color.lerp(cs.surfaceContainerLowest, cs.primary, 0.09)!,
                      cs.surfaceContainerLow,
                    ]
                  : [
                      Colors.white,
                      Color.lerp(Colors.white, cs.primaryContainer, 0.18)!,
                      cs.surfaceContainerLow,
                    ],
            ),
            borderRadius: BorderRadius.circular(28),
            border: Border.all(
              color: isDark
                  ? cs.primary.withValues(alpha: 0.3)
                  : cs.primary.withValues(alpha: 0.22),
              width: 1.5,
            ),
            boxShadow: [
              BoxShadow(
                color: cs.primary.withValues(alpha: isDark ? 0.16 : 0.08),
                blurRadius: 36,
                offset: const Offset(0, 10),
              ),
            ],
          ),
          heroCardGradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: isDark
                ? [
                    cs.surfaceContainerHigh.withValues(alpha: 0.95),
                    cs.surfaceContainer.withValues(alpha: 0.85),
                  ]
                : [
                    Colors.white.withValues(alpha: 0.95),
                    cs.surfaceContainerLowest.withValues(alpha: 0.88),
                  ],
          ),
          heroCardBorder: isDark
              ? cs.primary.withValues(alpha: 0.35)
              : cs.outlineVariant.withValues(alpha: 0.65),
          heroNumberGradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: isDark
                ? [
                    Colors.white,
                    cs.primaryContainer,
                    Color.lerp(cs.primaryContainer, cs.primary, 0.5)!,
                  ]
                : [
                    cs.onSurface,
                    Color.lerp(cs.onSurface, cs.primary, 0.7)!,
                    cs.primary,
                  ],
          ),
          cardGradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: isDark
                ? [
                    cs.surfaceContainer.withValues(alpha: 0.85),
                    cs.surfaceContainerLow.withValues(alpha: 0.70),
                  ]
                : [
                    Colors.white.withValues(alpha: 0.92),
                    cs.surfaceContainerLow.withValues(alpha: 0.80),
                  ],
          ),
          cardBorder: cs.outlineVariant.withValues(alpha: 0.45),
          trendBarGradient: LinearGradient(
            begin: Alignment.bottomCenter,
            end: Alignment.topCenter,
            colors: [
              cs.primary.withValues(alpha: 0.45),
              cs.primary,
            ],
          ),
          primary: cs.primary,
          secondary: cs.secondary,
          accent: cs.tertiary,
          textPrimary: cs.onSurface,
          textSecondary: cs.onSurfaceVariant,
          textMuted: cs.outline,
          pillBackground: cs.primaryContainer.withValues(alpha: 0.45),
          pillBorder: cs.primary.withValues(alpha: 0.28),
          glowColor: cs.primary,
          isDark: isDark,
        );

      case ShareCardStyle.midnightDark:
        return ShareCardPalette(
          backgroundDecoration: BoxDecoration(
            gradient: const LinearGradient(
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
              stops: [0.0, 0.45, 1.0],
              colors: [
                Color(0xFF090D16),
                Color(0xFF0F172A),
                Color(0xFF131B2E),
              ],
            ),
            borderRadius: BorderRadius.circular(28),
            border: Border.all(
              color: const Color(0xFF38BDF8).withValues(alpha: 0.35),
              width: 1.5,
            ),
            boxShadow: [
              BoxShadow(
                color: const Color(0xFF0284C7).withValues(alpha: 0.12),
                blurRadius: 40,
                offset: const Offset(0, 12),
              ),
            ],
          ),
          heroCardGradient: const LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              Color(0xFF1E293B),
              Color(0xFF0F172A),
            ],
          ),
          heroCardBorder: const Color(0xFF38BDF8).withValues(alpha: 0.35),
          heroNumberGradient: const LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              Colors.white,
              Color(0xFFF1F5F9),
              Color(0xFF7DD3FC), // Electric ice cyan
            ],
          ),
          cardGradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              const Color(0xFF1E293B).withValues(alpha: 0.88),
              const Color(0xFF0F172A).withValues(alpha: 0.78),
            ],
          ),
          cardBorder: const Color(0xFF334155),
          trendBarGradient: const LinearGradient(
            begin: Alignment.bottomCenter,
            end: Alignment.topCenter,
            colors: [
              Color(0xFF0369A1),
              Color(0xFF38BDF8),
            ],
          ),
          primary: const Color(0xFF38BDF8),
          secondary: const Color(0xFF818CF8),
          accent: const Color(0xFF34D399),
          textPrimary: const Color(0xFFF8FAFC),
          textSecondary: const Color(0xFFCBD5E1),
          textMuted: const Color(0xFF94A3B8),
          pillBackground: const Color(0xFF0F172A),
          pillBorder: const Color(0xFF38BDF8).withValues(alpha: 0.35),
          glowColor: const Color(0xFF38BDF8),
          isDark: true,
        );

      case ShareCardStyle.auroraGlow:
        return ShareCardPalette(
          backgroundDecoration: BoxDecoration(
            gradient: const LinearGradient(
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
              stops: [0.0, 0.45, 1.0],
              colors: [
                Color(0xFF16092A),
                Color(0xFF220E40),
                Color(0xFF0E1326),
              ],
            ),
            borderRadius: BorderRadius.circular(28),
            border: Border.all(
              color: const Color(0xFFA855F7).withValues(alpha: 0.45),
              width: 1.5,
            ),
            boxShadow: [
              BoxShadow(
                color: const Color(0xFFA855F7).withValues(alpha: 0.18),
                blurRadius: 40,
                offset: const Offset(0, 12),
              ),
            ],
          ),
          heroCardGradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              const Color(0xFF3B1D66).withValues(alpha: 0.75),
              const Color(0xFF1B0E33).withValues(alpha: 0.85),
            ],
          ),
          heroCardBorder: const Color(0xFFA855F7).withValues(alpha: 0.4),
          heroNumberGradient: const LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              Colors.white,
              Color(0xFFF3E8FF),
              Color(0xFFC084FC), // Luminous lavender
            ],
          ),
          cardGradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              const Color(0xFF2D164D).withValues(alpha: 0.60),
              const Color(0xFF150B26).withValues(alpha: 0.70),
            ],
          ),
          cardBorder: const Color(0xFFA855F7).withValues(alpha: 0.32),
          trendBarGradient: const LinearGradient(
            begin: Alignment.bottomCenter,
            end: Alignment.topCenter,
            colors: [
              Color(0xFF6B21A8),
              Color(0xFFC084FC),
            ],
          ),
          primary: const Color(0xFFC084FC),
          secondary: const Color(0xFFF472B6),
          accent: const Color(0xFF38BDF8),
          textPrimary: Colors.white,
          textSecondary: const Color(0xFFE2E8F0),
          textMuted: const Color(0xFFA78BFA),
          pillBackground: const Color(0xFF4C1D95).withValues(alpha: 0.5),
          pillBorder: const Color(0xFFC084FC).withValues(alpha: 0.4),
          glowColor: const Color(0xFFA855F7),
          isDark: true,
        );

      case ShareCardStyle.crispMinimal:
        return ShareCardPalette(
          backgroundDecoration: BoxDecoration(
            gradient: const LinearGradient(
              begin: Alignment.topCenter,
              end: Alignment.bottomCenter,
              colors: [
                Color(0xFFFFFFFF),
                Color(0xFFF8FAFC),
                Color(0xFFF1F5F9),
              ],
            ),
            borderRadius: BorderRadius.circular(28),
            border: Border.all(
              color: const Color(0xFFCBD5E1),
              width: 1.5,
            ),
            boxShadow: [
              BoxShadow(
                color: Colors.black.withValues(alpha: 0.05),
                blurRadius: 28,
                offset: const Offset(0, 8),
              ),
            ],
          ),
          heroCardGradient: const LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              Colors.white,
              Color(0xFFF8FAFC),
            ],
          ),
          heroCardBorder: const Color(0xFFE2E8F0),
          heroNumberGradient: const LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              Color(0xFF0F172A),
              Color(0xFF1E3A8A), // Deep royal navy
            ],
          ),
          cardGradient: const LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [
              Colors.white,
              Color(0xFFFAFAFA),
            ],
          ),
          cardBorder: const Color(0xFFE2E8F0),
          trendBarGradient: const LinearGradient(
            begin: Alignment.bottomCenter,
            end: Alignment.topCenter,
            colors: [
              Color(0xFF60A5FA),
              Color(0xFF2563EB),
            ],
          ),
          primary: const Color(0xFF2563EB),
          secondary: const Color(0xFF059669),
          accent: const Color(0xFFD97706),
          textPrimary: const Color(0xFF0F172A),
          textSecondary: const Color(0xFF475569),
          textMuted: const Color(0xFF94A3B8),
          pillBackground: const Color(0xFFEFF6FF),
          pillBorder: const Color(0xFF93C5FD),
          glowColor: const Color(0xFF2563EB),
          isDark: false,
        );
    }
  }
}

/// Standalone Share Stat Card Widget rendered for preview and image capture
class ShareStatCard extends StatelessWidget {
  final OverviewData data;
  final String periodLabel;
  final String? rangeKey;
  final ShareCardStyle style;
  final String? customHandle;
  final bool showCost;
  final bool showActivity;
  final bool showModels;
  final bool showTools;
  final bool showTrend;
  final bool showHeatmap;
  final ShareTrendChartType trendChartType;

  const ShareStatCard({
    super.key,
    required this.data,
    required this.periodLabel,
    this.rangeKey,
    this.style = ShareCardStyle.m3Dynamic,
    this.customHandle,
    this.showCost = true,
    this.showActivity = true,
    this.showModels = true,
    this.showTools = true,
    this.showTrend = true,
    this.showHeatmap = true,
    this.trendChartType = ShareTrendChartType.multiModel,
  });

  String get _effectiveRangeKey {
    if (rangeKey != null && rangeKey!.isNotEmpty) return rangeKey!;
    if (periodLabel.contains('今日') || periodLabel.toLowerCase().contains('today')) return 'today';
    if (periodLabel.contains('7') || periodLabel.contains('周') || periodLabel.toLowerCase().contains('week')) return 'week';
    if (periodLabel.contains('30') || periodLabel.contains('月') || periodLabel.toLowerCase().contains('month')) return 'month';
    if (periodLabel.contains('全') || periodLabel.toLowerCase().contains('all')) return 'all';
    if (data.rangeType.isNotEmpty) return data.rangeType;
    return 'week';
  }

  String get _reportSubTitle {
    switch (_effectiveRangeKey) {
      case 'today':
        return 'AI 用量追踪与成本分析日报';
      case 'week':
        return 'AI 用量追踪与成本分析周报';
      case 'month':
        return 'AI 用量追踪与成本分析月报';
      case 'all':
        return 'AI 用量追踪与成本全景总报';
      default:
        return 'AI 用量追踪与成本分析报告';
    }
  }

  String get _reportTag {
    switch (_effectiveRangeKey) {
      case 'today':
        return 'DAILY REPORT';
      case 'week':
        return 'WEEKLY REPORT';
      case 'month':
        return 'MONTHLY REPORT';
      case 'all':
        return 'ALL-TIME REPORT';
      default:
        return 'AI REPORT';
    }
  }

  String get _trendTitle {
    switch (_effectiveRangeKey) {
      case 'today':
        return '今日时段走势 (HOURLY RHYTHM)';
      case 'all':
        return '历史用量趋势 (ACTIVITY RHYTHM)';
      default:
        return '每日用量趋势 (ACTIVITY RHYTHM)';
    }
  }

  String get _trendBadge {
    if (data.daily.isEmpty) return '暂无数据';
    if (_effectiveRangeKey == 'today') {
      return '${data.daily.length} 个活跃时段';
    }
    if (_effectiveRangeKey == 'all') {
      return '${data.daily.length} 日记录';
    }
    return '${data.daily.length} 日走势';
  }

  @override
  Widget build(BuildContext context) {
    final appTheme = Theme.of(context);
    final themeProvider = Provider.of<ThemeProvider>(context, listen: false);
    final palette = ShareCardPalette.resolve(style, appTheme);
    final span = data.span;

    // Cache hit calculation
    final totalTokens = span.totalTokens;
    final cacheRead = span.cacheReadTokens;
    final cacheHitRate =
        totalTokens > 0 ? (cacheRead / totalTokens * 100).clamp(0.0, 100.0) : 0.0;

    // Top models (sorted by token count desc)
    final sortedModels = List<ShareRow>.from(data.byModel)
      ..sort((a, b) => b.tokens.compareTo(a.tokens));
    final topModels = sortedModels.take(4).toList();

    // Top tools
    final sortedTools = List<AppSummary>.from(data.byApp)
      ..sort((a, b) => b.totalTokens.compareTo(a.totalTokens));
    final topTools = sortedTools.take(5).toList();

    // Format generated date
    final genDateStr = DateFormat('yyyy.MM.dd HH:mm').format(DateTime.now());

    return Container(
      width: 640,
      decoration: palette.backgroundDecoration,
      clipBehavior: Clip.antiAlias,
      child: Stack(
        children: [
          // Ambient Atmospheric Lighting Glow Orbs
          Positioned(
            top: -40,
            right: 20,
            child: IgnorePointer(
              child: Container(
                width: 280,
                height: 280,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  gradient: RadialGradient(
                    colors: [
                      palette.glowColor
                          .withValues(alpha: palette.isDark ? 0.22 : 0.08),
                      palette.glowColor.withValues(alpha: 0.0),
                    ],
                  ),
                ),
              ),
            ),
          ),
          Positioned(
            bottom: 60,
            left: -30,
            child: IgnorePointer(
              child: Container(
                width: 240,
                height: 240,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  gradient: RadialGradient(
                    colors: [
                      palette.accent
                          .withValues(alpha: palette.isDark ? 0.14 : 0.05),
                      palette.accent.withValues(alpha: 0.0),
                    ],
                  ),
                ),
              ),
            ),
          ),

          // Main Card Structure
          Padding(
            padding: const EdgeInsets.all(28.0),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                // 1. Header: Branding, Period, Custom Handle
                Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    // Brand Icon with gradient sheen
                    Container(
                      width: 44,
                      height: 44,
                      decoration: BoxDecoration(
                        gradient: LinearGradient(
                          begin: Alignment.topLeft,
                          end: Alignment.bottomRight,
                          colors: [
                            palette.primary.withValues(alpha: 0.28),
                            palette.accent.withValues(alpha: 0.10),
                          ],
                        ),
                        borderRadius: BorderRadius.circular(14),
                        border: Border.all(
                          color: palette.primary.withValues(alpha: 0.4),
                          width: 1.2,
                        ),
                        boxShadow: [
                          BoxShadow(
                            color: palette.primary.withValues(
                                alpha: palette.isDark ? 0.2 : 0.08),
                            blurRadius: 12,
                            offset: const Offset(0, 4),
                          ),
                        ],
                      ),
                      child: Icon(
                        Icons.auto_awesome,
                        color: palette.primary,
                        size: 24,
                      ),
                    ),
                    const SizedBox(width: 14),
                    // App Name & Tagline
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Row(
                            children: [
                              Flexible(
                                child: Text(
                                  'GlobalTokenTracker++',
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: TextStyle(
                                    color: palette.textPrimary,
                                    fontWeight: FontWeight.w800,
                                    fontSize: 18,
                                    letterSpacing: -0.3,
                                  ),
                                ),
                              ),
                              const SizedBox(width: 8),
                              Container(
                                padding: const EdgeInsets.symmetric(
                                    horizontal: 8, vertical: 3),
                                decoration: BoxDecoration(
                                  gradient: LinearGradient(
                                    colors: [
                                      palette.pillBackground,
                                      palette.pillBackground
                                          .withValues(alpha: 0.7),
                                    ],
                                  ),
                                  borderRadius: BorderRadius.circular(20),
                                  border: Border.all(color: palette.pillBorder),
                                ),
                                child: Text(
                                  _reportTag,
                                  style: TextStyle(
                                    color: palette.primary,
                                    fontSize: 10,
                                    fontWeight: FontWeight.w700,
                                    letterSpacing: 0.5,
                                  ),
                                ),
                              ),
                            ],
                          ),
                          const SizedBox(height: 2),
                          Text(
                            _reportSubTitle,
                            style: TextStyle(
                              color: palette.textSecondary,
                              fontSize: 12,
                            ),
                          ),
                        ],
                      ),
                    ),
                    // Period & Date Tag
                    Column(
                      crossAxisAlignment: CrossAxisAlignment.end,
                      children: [
                        Container(
                          padding: const EdgeInsets.symmetric(
                              horizontal: 12, vertical: 6),
                          decoration: BoxDecoration(
                            gradient: LinearGradient(
                              colors: [
                                palette.primary.withValues(alpha: 0.16),
                                palette.primary.withValues(alpha: 0.06),
                              ],
                            ),
                            borderRadius: BorderRadius.circular(20),
                            border: Border.all(
                              color: palette.primary.withValues(alpha: 0.35),
                            ),
                          ),
                          child: Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              Icon(
                                Icons.calendar_today_outlined,
                                size: 13,
                                color: palette.primary,
                              ),
                              const SizedBox(width: 5),
                              Text(
                                periodLabel,
                                style: TextStyle(
                                  color: palette.primary,
                                  fontSize: 12,
                                  fontWeight: FontWeight.bold,
                                ),
                              ),
                            ],
                          ),
                        ),
                        if (customHandle != null &&
                            customHandle!.trim().isNotEmpty) ...[
                          const SizedBox(height: 4),
                          Text(
                            '@${customHandle!.trim().replaceAll('@', '')}',
                            style: TextStyle(
                              color: palette.textMuted,
                              fontSize: 11,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ],
                      ],
                    ),
                  ],
                ),
                const SizedBox(height: 24),

                // 2. Hero Big Stat Card: Total Tokens with metallic gradient typography & ambient glow
                Container(
                  padding: const EdgeInsets.all(22),
                  decoration: BoxDecoration(
                    gradient: palette.heroCardGradient,
                    borderRadius: BorderRadius.circular(22),
                    border: Border.all(color: palette.heroCardBorder),
                    boxShadow: [
                      BoxShadow(
                        color: palette.glowColor.withValues(
                            alpha: palette.isDark ? 0.22 : 0.08),
                        blurRadius: 24,
                        offset: const Offset(0, 8),
                      ),
                      BoxShadow(
                        color: Colors.black.withValues(
                            alpha: palette.isDark ? 0.25 : 0.04),
                        blurRadius: 10,
                        offset: const Offset(0, 2),
                      ),
                    ],
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          Row(
                            children: [
                              Icon(
                                Icons.token,
                                size: 16,
                                color: palette.primary,
                              ),
                              const SizedBox(width: 6),
                              Text(
                                'TOTAL TOKENS 消耗总量',
                                style: TextStyle(
                                  color: palette.textMuted,
                                  fontSize: 12,
                                  fontWeight: FontWeight.bold,
                                  letterSpacing: 0.8,
                                ),
                              ),
                            ],
                          ),
                          Container(
                            padding: const EdgeInsets.symmetric(
                                horizontal: 10, vertical: 4),
                            decoration: BoxDecoration(
                              gradient: LinearGradient(
                                colors: [
                                  palette.accent.withValues(alpha: 0.22),
                                  palette.accent.withValues(alpha: 0.08),
                                ],
                              ),
                              borderRadius: BorderRadius.circular(12),
                              border: Border.all(
                                  color: palette.accent
                                      .withValues(alpha: 0.3)),
                            ),
                            child: Text(
                              '⚡ 生产力全开',
                              style: TextStyle(
                                color: palette.accent,
                                fontSize: 11,
                                fontWeight: FontWeight.bold,
                              ),
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 10),
                      Row(
                        crossAxisAlignment: CrossAxisAlignment.baseline,
                        textBaseline: TextBaseline.alphabetic,
                        children: [
                          ShaderMask(
                            shaderCallback: (bounds) => palette
                                .heroNumberGradient
                                .createShader(bounds),
                            blendMode: BlendMode.srcIn,
                            child: Text(
                              themeProvider.formatTokens(span.totalTokens),
                              style: const TextStyle(
                                color: Colors.white,
                                fontSize: 44,
                                fontWeight: FontWeight.w900,
                                letterSpacing: -1.2,
                                height: 1.05,
                              ),
                            ),
                          ),
                          const SizedBox(width: 8),
                          Text(
                            'Tokens',
                            style: TextStyle(
                              color: palette.textSecondary,
                              fontSize: 18,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 16),
                      // Sub-breakdown pills: Input, Output, Cache/Reasoning
                      Wrap(
                        spacing: 8,
                        runSpacing: 8,
                        children: [
                          _buildTokenPill(
                            label: '输入 Input',
                            value: themeProvider
                                .formatTokens(span.inputTokens),
                            color: palette.primary,
                            palette: palette,
                          ),
                          _buildTokenPill(
                            label: '输出 Output',
                            value: themeProvider
                                .formatTokens(span.outputTokens),
                            color: palette.secondary,
                            palette: palette,
                          ),
                          if (span.cacheReadTokens > 0 ||
                              span.reasoningTokens > 0)
                            _buildTokenPill(
                              label: '缓存与推理',
                              value: themeProvider.formatTokens(
                                  span.cacheReadTokens +
                                      span.reasoningTokens),
                              color: palette.accent,
                              palette: palette,
                            ),
                        ],
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 16),

                // 3. Highlight Metrics Row (Cost, Requests, Duration, Cache)
                Row(
                  children: [
                    if (showCost)
                      Expanded(
                        child: _buildMetricTile(
                          title: '预估费用',
                          value: '\$${span.costUsd.toStringAsFixed(2)}',
                          subtext: span.credits > 0
                              ? '抵扣: ${span.credits.toStringAsFixed(1)}'
                              : '按官方价目折算',
                          icon: Icons.attach_money,
                          color: const Color(0xFF10B981),
                          palette: palette,
                        ),
                      ),
                    if (showCost && showActivity) const SizedBox(width: 12),
                    if (showActivity)
                      Expanded(
                        child: _buildMetricTile(
                          title: '请求频次',
                          value:
                              '${themeProvider.formatTokens(span.events)} 次',
                          subtext:
                              '活跃: ${(span.activeMs / 1000 / 60).toStringAsFixed(0)} 分钟',
                          icon: Icons.bolt,
                          color: const Color(0xFFF59E0B),
                          palette: palette,
                        ),
                      ),
                    if (showActivity) const SizedBox(width: 12),
                    Expanded(
                      child: _buildMetricTile(
                        title: '缓存命中率',
                        value: '${cacheHitRate.toStringAsFixed(1)}%',
                        subtext:
                            '命中: ${themeProvider.formatTokens(span.cacheReadTokens)}',
                        icon: Icons.speed,
                        color: const Color(0xFF8B5CF6),
                        palette: palette,
                      ),
                    ),
                  ],
                ),

                // 4. Token Activity Heatmap (Commit Matrix & Streaks)
                if (showHeatmap && data.activity.isNotEmpty) ...[
                  const SizedBox(height: 16),
                  _buildHeatmapSection(
                    data.activity,
                    palette,
                    themeProvider,
                  ),
                ],

                // 5. Daily Trend Sparkline / Multi-Model Trend Chart
                if (showTrend && data.daily.isNotEmpty) ...[
                  const SizedBox(height: 16),
                  Container(
                    padding: const EdgeInsets.all(18),
                    decoration: BoxDecoration(
                      gradient: palette.cardGradient,
                      borderRadius: BorderRadius.circular(20),
                      border: Border.all(color: palette.cardBorder),
                      boxShadow: [
                        BoxShadow(
                          color: Colors.black.withValues(
                              alpha: palette.isDark ? 0.15 : 0.03),
                          blurRadius: 10,
                          offset: const Offset(0, 3),
                        ),
                      ],
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          mainAxisAlignment: MainAxisAlignment.spaceBetween,
                          children: [
                            Expanded(
                              child: Row(
                                children: [
                                  Icon(
                                    trendChartType == ShareTrendChartType.multiModel
                                        ? Icons.insights_rounded
                                        : Icons.show_chart,
                                    size: 15,
                                    color: palette.primary,
                                  ),
                                  const SizedBox(width: 6),
                                  Flexible(
                                    child: Text(
                                      _trendTitle,
                                      overflow: TextOverflow.ellipsis,
                                      style: TextStyle(
                                        color: palette.textMuted,
                                        fontSize: 11,
                                        fontWeight: FontWeight.bold,
                                        letterSpacing: 0.5,
                                      ),
                                    ),
                                  ),
                                ],
                              ),
                            ),
                            const SizedBox(width: 8),
                            Text(
                              _trendBadge,
                              style: TextStyle(
                                color: palette.textSecondary,
                                fontSize: 11,
                              ),
                            ),
                          ],
                        ),
                        const SizedBox(height: 14),
                        if (trendChartType == ShareTrendChartType.multiModel)
                          _buildMultiModelTrend(
                              data.daily, palette, themeProvider)
                        else
                          _buildDailyBars(
                              data.daily, palette, themeProvider),
                      ],
                    ),
                  ),
                ],

                // 5. Top Models Breakdown with Gradient Progress Bars
                if (showModels && topModels.isNotEmpty) ...[
                  const SizedBox(height: 16),
                  Container(
                    padding: const EdgeInsets.all(18),
                    decoration: BoxDecoration(
                      gradient: palette.cardGradient,
                      borderRadius: BorderRadius.circular(20),
                      border: Border.all(color: palette.cardBorder),
                      boxShadow: [
                        BoxShadow(
                          color: Colors.black.withValues(
                              alpha: palette.isDark ? 0.15 : 0.03),
                          blurRadius: 10,
                          offset: const Offset(0, 3),
                        ),
                      ],
                    ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Row(
                          children: [
                            Icon(Icons.psychology_outlined,
                                size: 16, color: palette.secondary),
                            const SizedBox(width: 6),
                            Text(
                              '核心模型使用占比',
                              style: TextStyle(
                                color: palette.textMuted,
                                fontSize: 11,
                                fontWeight: FontWeight.bold,
                                letterSpacing: 0.5,
                              ),
                            ),
                          ],
                        ),
                        const SizedBox(height: 12),
                        ...topModels.map((m) {
                          final ratio = totalTokens > 0
                              ? (m.tokens / totalTokens).clamp(0.0, 1.0)
                              : 0.0;
                          final modelGrad = _getModelGradient(m.name, palette);
                          return Padding(
                            padding: const EdgeInsets.only(bottom: 8.0),
                            child: Column(
                              crossAxisAlignment:
                                  CrossAxisAlignment.start,
                              children: [
                                Row(
                                  mainAxisAlignment:
                                      MainAxisAlignment.spaceBetween,
                                  children: [
                                    Expanded(
                                      child: Text(
                                        m.name,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                        style: TextStyle(
                                          color: palette.textPrimary,
                                          fontSize: 12,
                                          fontWeight: FontWeight.w600,
                                        ),
                                      ),
                                    ),
                                    const SizedBox(width: 8),
                                    Text(
                                      '${themeProvider.formatTokens(m.tokens)} (${(ratio * 100).toStringAsFixed(1)}%)',
                                      style: TextStyle(
                                        color: palette.textSecondary,
                                        fontSize: 12,
                                        fontWeight: FontWeight.w500,
                                      ),
                                    ),
                                  ],
                                ),
                                const SizedBox(height: 5),
                                Container(
                                  height: 6,
                                  decoration: BoxDecoration(
                                    color: palette.cardBorder
                                        .withValues(alpha: 0.5),
                                    borderRadius: BorderRadius.circular(4),
                                  ),
                                  child: FractionallySizedBox(
                                    alignment: Alignment.centerLeft,
                                    widthFactor: ratio,
                                    child: Container(
                                      decoration: BoxDecoration(
                                        borderRadius:
                                            BorderRadius.circular(4),
                                        gradient: modelGrad,
                                        boxShadow: [
                                          BoxShadow(
                                            color: modelGrad.colors.last
                                                .withValues(alpha: 0.35),
                                            blurRadius: 4,
                                            offset: const Offset(0, 1),
                                          ),
                                        ],
                                      ),
                                    ),
                                  ),
                                ),
                              ],
                            ),
                          );
                        }),
                      ],
                    ),
                  ),
                ],

                // 6. Tools Ecosystem
                if (showTools && topTools.isNotEmpty) ...[
                  const SizedBox(height: 16),
                  Container(
                    padding: const EdgeInsets.symmetric(
                        horizontal: 18, vertical: 14),
                    decoration: BoxDecoration(
                      gradient: palette.cardGradient,
                      borderRadius: BorderRadius.circular(20),
                      border: Border.all(color: palette.cardBorder),
                      boxShadow: [
                        BoxShadow(
                          color: Colors.black.withValues(
                              alpha: palette.isDark ? 0.15 : 0.03),
                          blurRadius: 10,
                          offset: const Offset(0, 3),
                        ),
                      ],
                    ),
                    child: Row(
                      children: [
                        Icon(Icons.hub_outlined,
                            size: 16, color: palette.accent),
                        const SizedBox(width: 8),
                        Text(
                          '活跃工具: ',
                          style: TextStyle(
                            color: palette.textMuted,
                            fontSize: 11,
                            fontWeight: FontWeight.bold,
                          ),
                        ),
                        const SizedBox(width: 8),
                        Expanded(
                          child: Wrap(
                            spacing: 8,
                            runSpacing: 6,
                            children: topTools.map((tool) {
                              return Container(
                                padding: const EdgeInsets.symmetric(
                                    horizontal: 10, vertical: 4),
                                decoration: BoxDecoration(
                                  gradient: LinearGradient(
                                    colors: [
                                      palette.pillBackground,
                                      palette.pillBackground
                                          .withValues(alpha: 0.65),
                                    ],
                                  ),
                                  borderRadius: BorderRadius.circular(12),
                                  border: Border.all(
                                      color: palette.pillBorder),
                                ),
                                child: Text(
                                  '${tool.app} (${themeProvider.formatTokens(tool.totalTokens)})',
                                  style: TextStyle(
                                    color: palette.textPrimary,
                                    fontSize: 11,
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                              );
                            }).toList(),
                          ),
                        ),
                      ],
                    ),
                  ),
                ],

                const SizedBox(height: 20),
                // 7. Footer: Disclaimer & Watermark
                Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    Expanded(
                      child: Row(
                        children: [
                          Icon(
                            Icons.security,
                            size: 13,
                            color: palette.textMuted,
                          ),
                          const SizedBox(width: 5),
                          Flexible(
                            child: Text(
                              '本地安全统计 · 零外传隐私保护',
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                color: palette.textMuted,
                                fontSize: 11,
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(width: 8),
                    Text(
                      'Generated $genDateStr',
                      style: TextStyle(
                        color: palette.textMuted,
                        fontSize: 11,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildTokenPill({
    required String label,
    required String value,
    required Color color,
    required ShareCardPalette palette,
  }) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      decoration: BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [
            color.withValues(alpha: 0.16),
            color.withValues(alpha: 0.05),
          ],
        ),
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: color.withValues(alpha: 0.28)),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: 7,
            height: 7,
            decoration: BoxDecoration(
              gradient: RadialGradient(
                colors: [
                  color,
                  color.withValues(alpha: 0.6),
                ],
              ),
              shape: BoxShape.circle,
              boxShadow: [
                BoxShadow(
                  color: color.withValues(alpha: 0.4),
                  blurRadius: 4,
                ),
              ],
            ),
          ),
          const SizedBox(width: 6),
          Text(
            '$label: ',
            style: TextStyle(
              color: palette.textSecondary,
              fontSize: 12,
              fontWeight: FontWeight.w500,
            ),
          ),
          Text(
            value,
            style: TextStyle(
              color: palette.textPrimary,
              fontSize: 12,
              fontWeight: FontWeight.bold,
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildMetricTile({
    required String title,
    required String value,
    required String subtext,
    required IconData icon,
    required Color color,
    required ShareCardPalette palette,
  }) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      decoration: BoxDecoration(
        gradient: palette.cardGradient,
        borderRadius: BorderRadius.circular(18),
        border: Border.all(color: palette.cardBorder),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: palette.isDark ? 0.18 : 0.03),
            blurRadius: 12,
            offset: const Offset(0, 4),
          ),
        ],
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Expanded(
                child: Text(
                  title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: palette.textMuted,
                    fontSize: 11,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
              const SizedBox(width: 4),
              Container(
                padding: const EdgeInsets.all(5),
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    begin: Alignment.topLeft,
                    end: Alignment.bottomRight,
                    colors: [
                      color.withValues(alpha: 0.22),
                      color.withValues(alpha: 0.06),
                    ],
                  ),
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(color: color.withValues(alpha: 0.3)),
                ),
                child: Icon(icon, size: 14, color: color),
              ),
            ],
          ),
          const SizedBox(height: 6),
          FittedBox(
            fit: BoxFit.scaleDown,
            alignment: Alignment.centerLeft,
            child: Text(
              value,
              style: TextStyle(
                color: palette.textPrimary,
                fontSize: 18,
                fontWeight: FontWeight.w800,
                letterSpacing: -0.3,
              ),
            ),
          ),
          const SizedBox(height: 3),
          Text(
            subtext,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: palette.textSecondary,
              fontSize: 10,
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildDailyBars(
    List<TrendBucket> daily,
    ShareCardPalette palette,
    ThemeProvider themeProvider,
  ) {
    final recentDays = daily.length > 14
        ? daily.sublist(daily.length - 14)
        : daily;
    int maxTokens = 1;
    for (var d in recentDays) {
      if (d.tokens > maxTokens) maxTokens = d.tokens;
    }

    return SizedBox(
      height: 60,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: recentDays.map((d) {
          final barRatio = (d.tokens / maxTokens).clamp(0.05, 1.0);
          final barHeight = 44 * barRatio;
          final dateParts = d.date.split('-');
          final label = dateParts.length >= 3
              ? '${dateParts[1]}/${dateParts[2]}'
              : d.date;

          return Expanded(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 2.0),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  Container(
                    height: barHeight,
                    decoration: BoxDecoration(
                      gradient: d.tokens > 0 ? palette.trendBarGradient : null,
                      color: d.tokens > 0 ? null : palette.cardBorder,
                      borderRadius: BorderRadius.circular(4),
                      boxShadow: d.tokens > 0
                          ? [
                              BoxShadow(
                                color: palette.primary.withValues(
                                    alpha: palette.isDark ? 0.35 : 0.15),
                                blurRadius: 4,
                                offset: const Offset(0, -1),
                              ),
                            ]
                          : null,
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.clip,
                    style: TextStyle(
                      color: palette.textMuted,
                      fontSize: 8,
                    ),
                  ),
                ],
              ),
            ),
          );
        }).toList(),
      ),
    );
  }

  LinearGradient _getModelGradient(String modelName, ShareCardPalette palette) {
    final lower = modelName.toLowerCase();
    if (lower.contains('claude') ||
        lower.contains('sonnet') ||
        lower.contains('haiku')) {
      return const LinearGradient(
        colors: [Color(0xFFF59E0B), Color(0xFFD97706)],
      );
    } else if (lower.contains('gemini')) {
      return const LinearGradient(
        colors: [Color(0xFF38BDF8), Color(0xFF1A73E8)],
      );
    } else if (lower.contains('gpt') ||
        lower.contains('o1') ||
        lower.contains('o3')) {
      return const LinearGradient(
        colors: [Color(0xFF34D399), Color(0xFF059669)],
      );
    } else if (lower.contains('deepseek')) {
      return const LinearGradient(
        colors: [Color(0xFF818CF8), Color(0xFF4F46E5)],
      );
    }
    return LinearGradient(
      colors: [palette.secondary, palette.primary],
    );
  }

  Color _getModelColor(String modelName, int index, ShareCardPalette palette) {
    final lower = modelName.toLowerCase();
    if (lower.contains('claude') || lower.contains('sonnet') || lower.contains('haiku')) {
      return const Color(0xFFF59E0B);
    } else if (lower.contains('gemini')) {
      return const Color(0xFF38BDF8);
    } else if (lower.contains('gpt') || lower.contains('o1') || lower.contains('o3')) {
      return const Color(0xFF10B981);
    } else if (lower.contains('deepseek')) {
      return const Color(0xFF818CF8);
    }
    const fallbackPalette = [
      Color(0xFF2979FF),
      Color(0xFF00E676),
      Color(0xFFFF5252),
      Color(0xFFFF9100),
      Color(0xFFB388FF),
      Color(0xFF00E5FF),
      Color(0xFFFF4081),
      Color(0xFF7C4DFF),
    ];
    return fallbackPalette[index % fallbackPalette.length];
  }

  double _computeNiceInterval(double rawMax) {
    if (rawMax <= 0) return 100.0;
    final roughInterval = (rawMax * 1.15) / 3.0;
    final exponent = (math.log(roughInterval) / math.ln10).floor();
    final magnitude = math.pow(10, exponent).toDouble();
    final fraction = roughInterval / magnitude;

    double niceInterval;
    if (fraction <= 1.5) {
      niceInterval = 1.0 * magnitude;
    } else if (fraction <= 3.0) {
      niceInterval = 2.0 * magnitude;
    } else if (fraction <= 7.0) {
      niceInterval = 5.0 * magnitude;
    } else {
      niceInterval = 10.0 * magnitude;
    }
    return niceInterval;
  }

  Widget _buildMultiModelTrend(
    List<TrendBucket> daily,
    ShareCardPalette palette,
    ThemeProvider themeProvider,
  ) {
    final recentBuckets = daily.length > 30
        ? daily.sublist(daily.length - 30)
        : daily;
    if (recentBuckets.isEmpty) {
      return const SizedBox.shrink();
    }

    final int numPoints = recentBuckets.length;

    // Collect distinct models
    final Map<String, int> modelTotalTokens = {};
    for (final b in recentBuckets) {
      for (final entry in b.top) {
        if (entry.value > 0) {
          modelTotalTokens[entry.key] =
              (modelTotalTokens[entry.key] ?? 0) + entry.value;
        }
      }
    }

    final sortedModels = modelTotalTokens.keys.toList()
      ..sort((a, b) =>
          (modelTotalTokens[b] ?? 0).compareTo(modelTotalTokens[a] ?? 0));
    final topModels = sortedModels.take(4).toList();

    double maxVal = 0;
    final List<LineChartBarData> lineBarsData = [];

    if (topModels.isNotEmpty) {
      for (int mIdx = 0; mIdx < topModels.length; mIdx++) {
        final model = topModels[mIdx];
        final color = _getModelColor(model, mIdx, palette);
        final spots = <FlSpot>[];

        for (int i = 0; i < numPoints; i++) {
          final bucket = recentBuckets[i];
          int val = 0;
          for (final entry in bucket.top) {
            if (entry.key == model) {
              val = entry.value;
              break;
            }
          }
          if (val > maxVal) maxVal = val.toDouble();
          spots.add(FlSpot(i.toDouble(), val.toDouble()));
        }

        if (numPoints == 1) {
          spots.add(FlSpot(1.0, spots.first.y));
        }

        lineBarsData.add(
          LineChartBarData(
            spots: spots,
            isCurved: true,
            curveSmoothness: 0.35,
            preventCurveOverShooting: true,
            color: color,
            barWidth: 2.2,
            isStrokeCapRound: true,
            dotData: FlDotData(
              show: numPoints <= 8,
              getDotPainter: (spot, percent, barData, index) =>
                  FlDotCirclePainter(
                radius: 3,
                color: color,
                strokeWidth: 1.5,
                strokeColor: palette.isDark ? const Color(0xFF0F172A) : Colors.white,
              ),
            ),
            belowBarData: BarAreaData(
              show: true,
              color: color.withValues(alpha: 0.07),
            ),
          ),
        );
      }
    } else {
      // Fallback: Total Tokens single curve
      final spots = <FlSpot>[];
      for (int i = 0; i < numPoints; i++) {
        final t = recentBuckets[i].tokens.toDouble();
        if (t > maxVal) maxVal = t;
        spots.add(FlSpot(i.toDouble(), t));
      }

      if (numPoints == 1) {
        spots.add(FlSpot(1.0, spots.first.y));
      }

      lineBarsData.add(
        LineChartBarData(
          spots: spots,
          isCurved: true,
          curveSmoothness: 0.35,
          preventCurveOverShooting: true,
          color: palette.primary,
          barWidth: 2.5,
          isStrokeCapRound: true,
          dotData: FlDotData(
            show: numPoints <= 10,
            getDotPainter: (spot, percent, barData, index) =>
                FlDotCirclePainter(
              radius: 3.5,
              color: palette.primary,
              strokeWidth: 1.5,
              strokeColor: palette.isDark ? const Color(0xFF0F172A) : Colors.white,
            ),
          ),
          belowBarData: BarAreaData(
            show: true,
            gradient: LinearGradient(
              begin: Alignment.topCenter,
              end: Alignment.bottomCenter,
              colors: [
                palette.primary.withValues(alpha: 0.22),
                palette.primary.withValues(alpha: 0.02),
              ],
            ),
          ),
        ),
      );
    }

    final double interval = _computeNiceInterval(maxVal);
    final double maxY = maxVal <= 0 ? 100 : (maxVal * 1.18);
    final double bottomInterval = math.max(1.0, (numPoints / 6).floorToDouble());

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (topModels.isNotEmpty) ...[
          Wrap(
            spacing: 12,
            runSpacing: 6,
            children: topModels.map((m) {
              final color = _getModelColor(m, topModels.indexOf(m), palette);
              final total = modelTotalTokens[m] ?? 0;
              return Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Container(
                    width: 7,
                    height: 7,
                    decoration: BoxDecoration(
                      color: color,
                      shape: BoxShape.circle,
                    ),
                  ),
                  const SizedBox(width: 5),
                  Text(
                    m,
                    style: TextStyle(
                      color: palette.textPrimary,
                      fontSize: 10,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(width: 3),
                  Text(
                    '(${themeProvider.formatTokens(total)})',
                    style: TextStyle(
                      color: palette.textMuted,
                      fontSize: 9,
                    ),
                  ),
                ],
              );
            }).toList(),
          ),
          const SizedBox(height: 10),
        ],
        SizedBox(
          height: 110,
          child: LineChart(
            LineChartData(
              lineTouchData: const LineTouchData(enabled: false),
              minY: 0,
              maxY: maxY,
              minX: 0,
              maxX: math.max(1.0, (numPoints - 1).toDouble()),
              gridData: FlGridData(
                show: true,
                drawVerticalLine: false,
                horizontalInterval: interval > 0 ? interval : 100,
                getDrawingHorizontalLine: (value) => FlLine(
                  color: palette.cardBorder.withValues(alpha: 0.45),
                  strokeWidth: 1,
                  dashArray: [4, 4],
                ),
              ),
              titlesData: FlTitlesData(
                show: true,
                topTitles: const AxisTitles(
                  sideTitles: SideTitles(showTitles: false),
                ),
                rightTitles: const AxisTitles(
                  sideTitles: SideTitles(showTitles: false),
                ),
                leftTitles: AxisTitles(
                  sideTitles: SideTitles(
                    showTitles: true,
                    reservedSize: 42,
                    interval: interval > 0 ? interval : 100,
                    getTitlesWidget: (value, meta) {
                      if (value < 0 || value > maxY) {
                        return const SizedBox.shrink();
                      }
                      return Padding(
                        padding: const EdgeInsets.only(right: 6.0),
                        child: Text(
                          themeProvider.formatTokens(value.toInt()),
                          textAlign: TextAlign.right,
                          style: TextStyle(
                            color: palette.textMuted,
                            fontSize: 9,
                            fontWeight: FontWeight.w500,
                          ),
                        ),
                      );
                    },
                  ),
                ),
                bottomTitles: AxisTitles(
                  sideTitles: SideTitles(
                    showTitles: true,
                    reservedSize: 18,
                    interval: bottomInterval,
                    getTitlesWidget: (value, meta) {
                      final idx = value.toInt();
                      if (idx < 0 || idx >= numPoints) {
                        return const SizedBox.shrink();
                      }
                      final b = recentBuckets[idx];
                      final dateParts = b.date.split('-');
                      final label = dateParts.length >= 3
                          ? '${dateParts[1]}/${dateParts[2]}'
                          : b.date;
                      return Padding(
                        padding: const EdgeInsets.only(top: 4.0),
                        child: Text(
                          label,
                          style: TextStyle(
                            color: palette.textMuted,
                            fontSize: 8.5,
                            fontWeight: FontWeight.w500,
                          ),
                        ),
                      );
                    },
                  ),
                ),
              ),
              borderData: FlBorderData(show: false),
              lineBarsData: lineBarsData,
            ),
          ),
        ),
      ],
    );
  }

  Widget _buildHeatmapSection(
    List<ActivityDay> activity,
    ShareCardPalette palette,
    ThemeProvider themeProvider,
  ) {
    final Map<String, ActivityDay> activityMap = {
      for (final a in activity) a.date: a,
    };

    int activeDays = 0;
    int totalTokens = 0;
    int maxDailyTokens = 0;

    for (final a in activity) {
      if (a.tokens > 0) {
        activeDays++;
        totalTokens += a.tokens;
        if (a.tokens > maxDailyTokens) {
          maxDailyTokens = a.tokens;
        }
      }
    }

    final sortedActiveDates = activity
        .where((a) => a.tokens > 0)
        .map((a) => a.date)
        .toList()
      ..sort();

    int longestStreak = 0;
    int currentStreak = 0;
    DateTime? prevDate;

    for (final dStr in sortedActiveDates) {
      try {
        final dt = DateTime.parse(dStr);
        if (prevDate != null) {
          final diff = dt.difference(prevDate).inDays;
          if (diff == 1) {
            currentStreak++;
          } else if (diff > 1) {
            currentStreak = 1;
          }
        } else {
          currentStreak = 1;
        }
        if (currentStreak > longestStreak) {
          longestStreak = currentStreak;
        }
        prevDate = dt;
      } catch (_) {}
    }

    const totalWeeks = 52;
    const daysPerWeek = 7;
    final now = DateTime.now();
    final today = DateTime(now.year, now.month, now.day);
    final currentDayOfWeek = today.weekday % 7;
    final startDate = today
        .subtract(Duration(days: (totalWeeks - 1) * 7 + currentDayOfWeek));

    final gridDates = List.generate(totalWeeks, (w) {
      return List.generate(daysPerWeek, (d) {
        return startDate.add(Duration(days: w * 7 + d));
      });
    });

    final Map<int, String> monthLabels = {};
    int lastMonth = -1;
    int lastWeekCol = -10;
    for (int w = 0; w < totalWeeks; w++) {
      final firstDayInWeek = gridDates[w][0];
      final month = firstDayInWeek.month;
      if (month != lastMonth) {
        if (lastWeekCol < 0 || (w - lastWeekCol) >= 3) {
          monthLabels[w] = '$month月';
          lastMonth = month;
          lastWeekCol = w;
        }
      }
    }

    const double cellSize = 8.0;
    const double cellSpacing = 2.0;

    return Container(
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        gradient: palette.cardGradient,
        borderRadius: BorderRadius.circular(20),
        border: Border.all(color: palette.cardBorder),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(
                alpha: palette.isDark ? 0.15 : 0.03),
            blurRadius: 10,
            offset: const Offset(0, 3),
          ),
        ],
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Row(
                children: [
                  Icon(
                    Icons.local_fire_department_rounded,
                    size: 16,
                    color: palette.accent,
                  ),
                  const SizedBox(width: 6),
                  Text(
                    'TOKEN 活动矩阵 (COMMIT HEATMAP)',
                    style: TextStyle(
                      color: palette.textMuted,
                      fontSize: 11,
                      fontWeight: FontWeight.bold,
                      letterSpacing: 0.5,
                    ),
                  ),
                ],
              ),
              Row(
                children: [
                  if (longestStreak > 1) ...[
                    Container(
                      padding: const EdgeInsets.symmetric(
                          horizontal: 8, vertical: 2),
                      decoration: BoxDecoration(
                        color: palette.pillBackground,
                        borderRadius: BorderRadius.circular(10),
                        border: Border.all(color: palette.pillBorder),
                      ),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Icon(
                            Icons.local_fire_department,
                            size: 11,
                            color: palette.accent,
                          ),
                          const SizedBox(width: 3),
                          Text(
                            '最长连续 $longestStreak 天',
                            style: TextStyle(
                              color: palette.textSecondary,
                              fontSize: 10,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(width: 8),
                  ],
                  Text(
                    '活跃 $activeDays 天',
                    style: TextStyle(
                      color: palette.textSecondary,
                      fontSize: 11,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ],
              ),
            ],
          ),
          const SizedBox(height: 14),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: List.generate(daysPerWeek, (d) {
                  String label = '';
                  if (d == 1) label = '一';
                  if (d == 3) label = '三';
                  if (d == 5) label = '五';
                  return Container(
                    width: 14,
                    height: cellSize,
                    margin: EdgeInsets.only(
                        bottom: d < daysPerWeek - 1 ? cellSpacing : 0),
                    alignment: Alignment.centerLeft,
                    child: Text(
                      label,
                      style: TextStyle(
                        color: palette.textMuted,
                        fontSize: 7.5,
                        height: 1.0,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                  );
                }),
              ),
              const SizedBox(width: 6),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: List.generate(totalWeeks, (w) {
                        return Padding(
                          padding: EdgeInsets.only(
                              right: w < totalWeeks - 1 ? cellSpacing : 0),
                          child: Column(
                            children: List.generate(daysPerWeek, (d) {
                              final dt = gridDates[w][d];
                              final isFuture = dt.isAfter(today);

                              if (isFuture) {
                                return Container(
                                  width: cellSize,
                                  height: cellSize,
                                  margin: EdgeInsets.only(
                                      bottom: d < daysPerWeek - 1
                                          ? cellSpacing
                                          : 0),
                                  color: Colors.transparent,
                                );
                              }

                              final dStr =
                                  DateFormat('yyyy-MM-dd').format(dt);
                              final act = activityMap[dStr];
                              final dailyTokens = act?.tokens ?? 0;
                              final level = _getHeatmapLevel(
                                  dailyTokens, maxDailyTokens);
                              final color =
                                  _getHeatmapCellColor(level, palette);

                              return Container(
                                width: cellSize,
                                height: cellSize,
                                margin: EdgeInsets.only(
                                    bottom: d < daysPerWeek - 1
                                        ? cellSpacing
                                        : 0),
                                decoration: BoxDecoration(
                                  color: color,
                                  borderRadius: BorderRadius.circular(2),
                                  border: Border.all(
                                    color: level == 0
                                        ? palette.cardBorder
                                            .withValues(alpha: 0.35)
                                        : Colors.transparent,
                                    width: 0.4,
                                  ),
                                ),
                              );
                            }),
                          ),
                        );
                      }),
                    ),
                    const SizedBox(height: 6),
                    SizedBox(
                      height: 14,
                      child: Stack(
                        children: monthLabels.entries.map((entry) {
                          final w = entry.key;
                          final label = entry.value;
                          final leftPos = w * (cellSize + cellSpacing);
                          return Positioned(
                            left: leftPos,
                            child: Text(
                              label,
                              style: TextStyle(
                                fontSize: 9,
                                color: palette.textMuted,
                                fontWeight: FontWeight.w500,
                              ),
                            ),
                          );
                        }).toList(),
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                '近一年累计活动活跃 · 产生 ${themeProvider.formatTokens(totalTokens)} Tokens',
                style: TextStyle(
                  color: palette.textMuted,
                  fontSize: 10,
                ),
              ),
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    '少',
                    style: TextStyle(color: palette.textMuted, fontSize: 10),
                  ),
                  const SizedBox(width: 4),
                  ...[0, 1, 2, 3, 4].map((level) => Container(
                        width: 8,
                        height: 8,
                        margin: const EdgeInsets.symmetric(horizontal: 1.5),
                        decoration: BoxDecoration(
                          color: _getHeatmapCellColor(level, palette),
                          borderRadius: BorderRadius.circular(2),
                          border: Border.all(
                            color: level == 0
                                ? palette.cardBorder.withValues(alpha: 0.5)
                                : Colors.transparent,
                            width: 0.5,
                          ),
                        ),
                      )),
                  const SizedBox(width: 4),
                  Text(
                    '多',
                    style: TextStyle(color: palette.textMuted, fontSize: 10),
                  ),
                ],
              ),
            ],
          ),
        ],
      ),
    );
  }

  int _getHeatmapLevel(int dailyTokens, int maxDailyTokens) {
    if (dailyTokens <= 0 || maxDailyTokens <= 0) return 0;
    final ratio = dailyTokens / maxDailyTokens;
    if (ratio <= 0.25) return 1;
    if (ratio <= 0.50) return 2;
    if (ratio <= 0.75) return 3;
    return 4;
  }

  Color _getHeatmapCellColor(int level, ShareCardPalette palette) {
    switch (level) {
      case 1:
        return palette.primary.withValues(alpha: palette.isDark ? 0.35 : 0.25);
      case 2:
        return palette.primary.withValues(alpha: palette.isDark ? 0.60 : 0.50);
      case 3:
        return palette.primary.withValues(alpha: palette.isDark ? 0.85 : 0.75);
      case 4:
        return palette.isDark ? palette.glowColor : palette.primary;
      case 0:
      default:
        return palette.isDark
            ? const Color(0xFF1E293B).withValues(alpha: 0.55)
            : const Color(0xFFE2E8F0).withValues(alpha: 0.8);
    }
  }
}

/// Full interactive Dialog for Customizing & Exporting the Share Stat Card
class ShareCardDialog extends StatefulWidget {
  final OverviewData initialData;
  final String currentRange;

  const ShareCardDialog({
    super.key,
    required this.initialData,
    required this.currentRange,
  });

  @override
  State<ShareCardDialog> createState() => _ShareCardDialogState();
}

class _ShareCardDialogState extends State<ShareCardDialog> {
  final GlobalKey _cardBoundaryKey = GlobalKey();
  final TextEditingController _handleController = TextEditingController();

  late String _selectedRange;
  late OverviewData _data;
  ShareCardStyle _selectedStyle = ShareCardStyle.m3Dynamic;

  bool _loadingRange = false;
  bool _showCost = true;
  bool _showActivity = true;
  bool _showModels = true;
  bool _showTools = true;
  bool _showTrend = true;
  bool _showHeatmap = true;
  ShareTrendChartType _trendChartType = ShareTrendChartType.multiModel;

  bool _isCopying = false;
  bool _copySuccess = false;

  @override
  void initState() {
    super.initState();
    _selectedRange = widget.currentRange;
    _data = widget.initialData;
  }

  @override
  void dispose() {
    _handleController.dispose();
    super.dispose();
  }

  String _getRangeLabel(String range) {
    switch (range) {
      case 'today':
        return '今日用量';
      case 'week':
        return '近 7 天统计';
      case 'month':
        return '近 30 天统计';
      case 'all':
        return '全部累计用量';
      default:
        return '周期用量统计';
    }
  }

  Future<void> _switchRange(String range) async {
    if (_selectedRange == range) return;
    setState(() {
      _selectedRange = range;
      _loadingRange = true;
    });

    try {
      final newData = await FfiBridge.instance.getOverview(rangeKey: range);
      if (mounted) {
        setState(() {
          _data = newData;
          _loadingRange = false;
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          _loadingRange = false;
        });
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('加载该周期数据失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  Future<Uint8List?> _captureCardPng() async {
    try {
      // Find the render object
      final boundary = _cardBoundaryKey.currentContext?.findRenderObject()
          as RenderRepaintBoundary?;
      if (boundary == null) return null;

      // Ultra crisp 3.0 pixel ratio rendering
      final ui.Image image = await boundary.toImage(pixelRatio: 3.0);
      final ByteData? byteData =
          await image.toByteData(format: ui.ImageByteFormat.png);
      return byteData?.buffer.asUint8List();
    } catch (e) {
      debugPrint('Failed to capture card: $e');
      return null;
    }
  }

  Future<void> _copyImageToClipboard() async {
    if (_isCopying) return;
    setState(() => _isCopying = true);

    try {
      final pngBytes = await _captureCardPng();
      if (pngBytes == null) {
        throw Exception('无法生成图片数据');
      }

      await Pasteboard.writeImage(pngBytes);

      if (mounted) {
        setState(() {
          _isCopying = false;
          _copySuccess = true;
        });

        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: const Row(
              children: [
                Icon(Icons.check_circle, color: Colors.white, size: 20),
                SizedBox(width: 8),
                Expanded(
                  child: Text(
                    '已成功复制卡片图片到剪贴板！可直接在微信/Slack/Twitter等软件中 Ctrl+V 粘贴',
                  ),
                ),
              ],
            ),
            backgroundColor: const Color(0xFF10B981),
            behavior: SnackBarBehavior.floating,
            duration: const Duration(seconds: 4),
          ),
        );

        Future.delayed(const Duration(seconds: 3), () {
          if (mounted) {
            setState(() => _copySuccess = false);
          }
        });
      }
    } catch (e) {
      if (mounted) {
        setState(() => _isCopying = false);
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('复制失败: $e'),
            backgroundColor: Colors.red,
          ),
        );
      }
    }
  }

  Future<void> _saveImageToFile() async {
    try {
      final pngBytes = await _captureCardPng();
      if (pngBytes == null) {
        throw Exception('无法生成图片数据');
      }

      final timeStr = DateFormat('yyyyMMdd_HHmm').format(DateTime.now());
      final defaultFileName = 'GlobalTokenTracker_${_selectedRange}_$timeStr.png';

      final savedUri = await FilePicker.saveFile(
        dialogTitle: '保存分享卡片图片',
        fileName: defaultFileName,
        bytes: pngBytes,
        type: FileType.custom,
        allowedExtensions: ['png'],
      );

      if (savedUri == null) return; // User cancelled
      final outputPath =
          savedUri.isScheme('file') ? savedUri.toFilePath() : savedUri.path;

      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('图片已成功保存到: $outputPath'),
            action: SnackBarAction(
              label: '打开所在目录',
              onPressed: () {
                if (Platform.isWindows) {
                  Process.run('explorer.exe', ['/select,', outputPath]);
                }
              },
            ),
            behavior: SnackBarBehavior.floating,
            duration: const Duration(seconds: 5),
          ),
        );
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('保存图片失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  Future<void> _quickSaveToPictures() async {
    try {
      final pngBytes = await _captureCardPng();
      if (pngBytes == null) throw Exception('无法生成图片数据');

      String? targetDir;
      if (Platform.isWindows) {
        final userProfile = Platform.environment['USERPROFILE'];
        if (userProfile != null) {
          final pictures = Directory('$userProfile\\Pictures');
          if (pictures.existsSync()) {
            targetDir = pictures.path;
          }
        }
      }

      targetDir ??= Directory.current.path;
      final timeStr = DateFormat('yyyyMMdd_HHmmss').format(DateTime.now());
      final targetPath = '$targetDir\\GlobalTokenTracker_${_selectedRange}_$timeStr.png';

      await File(targetPath).writeAsBytes(pngBytes);

      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('已快速保存到图片库: $targetPath'),
            action: SnackBarAction(
              label: '打开',
              onPressed: () {
                if (Platform.isWindows) {
                  Process.run('explorer.exe', ['/select,', targetPath]);
                }
              },
            ),
            behavior: SnackBarBehavior.floating,
          ),
        );
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('快速保存失败: $e'), backgroundColor: Colors.red),
        );
      }
    }
  }

  void _copyMarkdownSummary() {
    final themeProvider = Provider.of<ThemeProvider>(context, listen: false);
    final span = _data.span;
    final buffer = StringBuffer();
    final handle = _handleController.text.trim();

    final reportName = switch (_selectedRange) {
      'today' => '日报',
      'week' => '周报',
      'month' => '月报',
      'all' => '全景总报',
      _ => '用量报告',
    };
    buffer.writeln(
        '🤖 **GlobalTokenTracker++ AI $reportName**（${_getRangeLabel(_selectedRange)}）');
    if (handle.isNotEmpty) {
      buffer.writeln('👤 统计用户: @${handle.replaceAll('@', '')}');
    }
    buffer.writeln('📊 累计消耗: ${themeProvider.formatTokens(span.totalTokens)} Tokens '
        '(输入: ${themeProvider.formatTokens(span.inputTokens)} · 输出: ${themeProvider.formatTokens(span.outputTokens)})');
    if (_showCost) {
      buffer.writeln('💰 预估费用: \$${span.costUsd.toStringAsFixed(2)} USD');
    }
    if (_showActivity) {
      buffer.writeln('⚡ API 请求: ${themeProvider.formatTokens(span.events)} 次 · '
          '专注时长: ${(span.activeMs / 1000 / 60).toStringAsFixed(0)} 分钟');
    }

    if (_showHeatmap && _data.activity.isNotEmpty) {
      final activeDays = _data.activity.where((a) => a.tokens > 0).length;
      if (activeDays > 0) {
        buffer.writeln('📅 活跃天数: $activeDays 天活动打卡');
      }
    }

    if (_showModels && _data.byModel.isNotEmpty) {
      final top3 = _data.byModel.take(3).map((m) => m.name).join(', ');
      buffer.writeln('🏆 主要模型: $top3');
    }

    if (_showTools && _data.byApp.isNotEmpty) {
      final tools = _data.byApp.take(3).map((a) => a.app).join(', ');
      buffer.writeln('🛠️ 活跃工具: $tools');
    }
    final reportTag = switch (_selectedRange) {
      'today' => '#DailyReport',
      'week' => '#WeeklyReport',
      'month' => '#MonthlyReport',
      'all' => '#AllTimeReport',
      _ => '#AIReport',
    };
    buffer.writeln(
        '#GlobalTokenTracker #AI #LLM #TokenTracker $reportTag');

    Clipboard.setData(ClipboardData(text: buffer.toString()));
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(
        content: Text('已复制 Markdown 文本摘要到剪贴板！适合直接发推或动态'),
        behavior: SnackBarBehavior.floating,
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final size = MediaQuery.of(context).size;

    return Dialog(
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(28)),
      insetPadding: const EdgeInsets.symmetric(horizontal: 40, vertical: 28),
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: 1200,
          maxHeight: math.min(size.height * 0.92, 860),
        ),
        child: Column(
          children: [
            // Modal Top Header
            Padding(
              padding: const EdgeInsets.fromLTRB(24, 20, 16, 12),
              child: Row(
                children: [
                  Container(
                    padding: const EdgeInsets.all(8),
                    decoration: BoxDecoration(
                      color: theme.colorScheme.primaryContainer,
                      borderRadius: BorderRadius.circular(12),
                    ),
                    child: Icon(
                      Icons.share_outlined,
                      color: theme.colorScheme.onPrimaryContainer,
                      size: 20,
                    ),
                  ),
                  const SizedBox(width: 12),
                  Text(
                    '生成用量分享卡片',
                    style: theme.textTheme.titleLarge?.copyWith(
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                  const Spacer(),
                  IconButton(
                    onPressed: () => Navigator.of(context).pop(),
                    icon: const Icon(Icons.close),
                    tooltip: '关闭',
                  ),
                ],
              ),
            ),
            const Divider(height: 1),

            // Content Body: Left is Live Preview, Right is Config Panel
            Expanded(
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  // LEFT: Live Interactive Preview Canvas
                  Expanded(
                    flex: 6,
                    child: Container(
                      color: theme.colorScheme.surfaceContainerHighest
                          .withValues(alpha: 0.35),
                      padding: const EdgeInsets.all(24),
                      child: Center(
                        child: SingleChildScrollView(
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              // Non-intrusive loading overlay when changing range
                              if (_loadingRange)
                                const Padding(
                                  padding: EdgeInsets.only(bottom: 12.0),
                                  child: LinearProgressIndicator(),
                                ),

                              FittedBox(
                                fit: BoxFit.scaleDown,
                                alignment: Alignment.topCenter,
                                child: RepaintBoundary(
                                  key: _cardBoundaryKey,
                                  child: SizedBox(
                                    width: 640,
                                    child: ShareStatCard(
                                      data: _data,
                                      periodLabel: _getRangeLabel(_selectedRange),
                                      rangeKey: _selectedRange,
                                      style: _selectedStyle,
                                      customHandle: _handleController.text,
                                      showCost: _showCost,
                                      showActivity: _showActivity,
                                      showModels: _showModels,
                                      showTools: _showTools,
                                      showTrend: _showTrend,
                                      showHeatmap: _showHeatmap,
                                      trendChartType: _trendChartType,
                                    ),
                                  ),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ),
                  ),
                  const VerticalDivider(width: 1),

                  // RIGHT: Customization Controls & Action Buttons
                  Expanded(
                    flex: 5,
                    child: SingleChildScrollView(
                      padding: const EdgeInsets.all(24),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          // 1. Time Range Selector
                          Text(
                            '统计周期',
                            style: theme.textTheme.titleSmall?.copyWith(
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          const SizedBox(height: 8),
                          SegmentedButton<String>(
                            segments: const [
                              ButtonSegment(value: 'today', label: Text('今日')),
                              ButtonSegment(value: 'week', label: Text('近 7 天')),
                              ButtonSegment(value: 'month', label: Text('近 30 天')),
                              ButtonSegment(value: 'all', label: Text('全周期')),
                            ],
                            selected: {_selectedRange},
                            onSelectionChanged: (val) =>
                                _switchRange(val.first),
                          ),
                          const SizedBox(height: 20),

                          // 2. Style Preset Selector
                          Text(
                            '视觉卡片风格',
                            style: theme.textTheme.titleSmall?.copyWith(
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          const SizedBox(height: 8),
                          Wrap(
                            spacing: 8,
                            runSpacing: 8,
                            children: ShareCardStyle.values.map((s) {
                              final isSelected = _selectedStyle == s;
                              return ChoiceChip(
                                avatar: Icon(
                                  s.icon,
                                  size: 16,
                                  color: isSelected
                                      ? theme.colorScheme.onPrimary
                                      : null,
                                ),
                                label: Text(s.label),
                                selected: isSelected,
                                onSelected: (_) {
                                  setState(() => _selectedStyle = s);
                                },
                              );
                            }).toList(),
                          ),
                          const SizedBox(height: 20),

                          // 3. Custom Handle / User Tag
                          Text(
                            '作者昵称或社交 Handle (选填)',
                            style: theme.textTheme.titleSmall?.copyWith(
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          const SizedBox(height: 8),
                          TextField(
                            controller: _handleController,
                            onChanged: (_) => setState(() {}),
                            decoration: InputDecoration(
                              prefixIcon: const Icon(Icons.person_outline, size: 18),
                              hintText: '例如: developer 或 @username',
                              isDense: true,
                              border: OutlineInputBorder(
                                borderRadius: BorderRadius.circular(12),
                              ),
                            ),
                          ),
                          const SizedBox(height: 20),

                          // 4. Content Toggles
                          Text(
                            '卡片内容显示配置',
                            style: theme.textTheme.titleSmall?.copyWith(
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          const SizedBox(height: 4),
                          SwitchListTile(
                            contentPadding: EdgeInsets.zero,
                            title: const Text('显示预估成本金额'),
                            subtitle: const Text('可关闭以隐藏隐私费用'),
                            value: _showCost,
                            onChanged: (v) => setState(() => _showCost = v),
                          ),
                          SwitchListTile(
                            contentPadding: EdgeInsets.zero,
                            title: const Text('显示活跃时长与请求次数'),
                            value: _showActivity,
                            onChanged: (v) => setState(() => _showActivity = v),
                          ),
                          SwitchListTile(
                            contentPadding: EdgeInsets.zero,
                            title: const Text('显示 Token 活动打卡热力图'),
                            subtitle: const Text('展示类似 GitHub 贡献矩阵的年度活跃打卡'),
                            value: _showHeatmap,
                            onChanged: (v) => setState(() => _showHeatmap = v),
                          ),
                          SwitchListTile(
                            contentPadding: EdgeInsets.zero,
                            title: const Text('显示用量走势图表'),
                            subtitle: const Text('展示周期内的模型用量变化走势'),
                            value: _showTrend,
                            onChanged: (v) => setState(() => _showTrend = v),
                          ),
                          if (_showTrend) ...[
                            Padding(
                              padding: const EdgeInsets.only(
                                  left: 12.0, bottom: 8.0, top: 2.0),
                              child: Row(
                                children: [
                                  Text(
                                    '走势样式: ',
                                    style: TextStyle(
                                      fontSize: 12,
                                      color: theme.colorScheme.onSurfaceVariant,
                                      fontWeight: FontWeight.w600,
                                    ),
                                  ),
                                  const SizedBox(width: 8),
                                  SegmentedButton<ShareTrendChartType>(
                                    segments:
                                        ShareTrendChartType.values.map((t) {
                                      return ButtonSegment<ShareTrendChartType>(
                                        value: t,
                                        icon: Icon(t.icon, size: 14),
                                        label: Text(t.label,
                                            style:
                                                const TextStyle(fontSize: 12)),
                                      );
                                    }).toList(),
                                    selected: {_trendChartType},
                                    onSelectionChanged: (val) => setState(
                                        () => _trendChartType = val.first),
                                  ),
                                ],
                              ),
                            ),
                          ],
                          SwitchListTile(
                            contentPadding: EdgeInsets.zero,
                            title: const Text('显示核心模型分布排行'),
                            value: _showModels,
                            onChanged: (v) => setState(() => _showModels = v),
                          ),
                          SwitchListTile(
                            contentPadding: EdgeInsets.zero,
                            title: const Text('显示活跃工具生态徽章'),
                            value: _showTools,
                            onChanged: (v) => setState(() => _showTools = v),
                          ),
                          const SizedBox(height: 24),

                          // 5. Primary Action Buttons
                          SizedBox(
                            width: double.infinity,
                            height: 48,
                            child: FilledButton.icon(
                              onPressed: _isCopying ? null : _copyImageToClipboard,
                              icon: _copySuccess
                                  ? const Icon(Icons.check, size: 20)
                                  : (_isCopying
                                      ? const SizedBox(
                                          width: 18,
                                          height: 18,
                                          child: CircularProgressIndicator(
                                            strokeWidth: 2,
                                            color: Colors.white,
                                          ),
                                        )
                                      : const Icon(Icons.copy, size: 20)),
                              label: Text(
                                _copySuccess
                                    ? '已成功复制图片！'
                                    : (_isCopying ? '正在生成图片...' : '复制图片到剪贴板'),
                                style: const TextStyle(
                                  fontWeight: FontWeight.bold,
                                  fontSize: 15,
                                ),
                              ),
                            ),
                          ),
                          const SizedBox(height: 10),
                          Row(
                            children: [
                              Expanded(
                                child: OutlinedButton.icon(
                                  onPressed: _saveImageToFile,
                                  icon: const Icon(Icons.save_alt, size: 18),
                                  label: const Text('另存为图片...'),
                                ),
                              ),
                              const SizedBox(width: 8),
                              Expanded(
                                child: OutlinedButton.icon(
                                  onPressed: _quickSaveToPictures,
                                  icon: const Icon(Icons.photo_library_outlined,
                                      size: 18),
                                  label: const Text('快速存入相册'),
                                ),
                              ),
                            ],
                          ),
                          const SizedBox(height: 8),
                          Center(
                            child: TextButton.icon(
                              onPressed: _copyMarkdownSummary,
                              icon: const Icon(Icons.notes, size: 16),
                              label: const Text('复制 Markdown 文本摘要'),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Helper function to display the share card dialog
void showShareCardDialog(
  BuildContext context,
  OverviewData data,
  String currentRange,
) {
  showDialog(
    context: context,
    builder: (ctx) => ShareCardDialog(
      initialData: data,
      currentRange: currentRange,
    ),
  );
}
