class Totals {
  final int events;
  final int inputTokens;
  final int outputTokens;
  final int reasoningTokens;
  final int cacheReadTokens;
  final int cacheWriteTokens;
  final double credits;
  final double costUsd;
  final int activeMs;

  Totals({
    required this.events,
    required this.inputTokens,
    required this.outputTokens,
    required this.reasoningTokens,
    required this.cacheReadTokens,
    required this.cacheWriteTokens,
    required this.credits,
    required this.costUsd,
    required this.activeMs,
  });

  int get totalTokens => inputTokens + outputTokens + cacheReadTokens;

  factory Totals.fromJson(Map<String, dynamic> json) {
    return Totals(
      events: json['events'] ?? 0,
      inputTokens: json['input_tokens'] ?? 0,
      outputTokens: json['output_tokens'] ?? 0,
      reasoningTokens: json['reasoning_tokens'] ?? 0,
      cacheReadTokens: json['cache_read_tokens'] ?? 0,
      cacheWriteTokens: json['cache_write_tokens'] ?? 0,
      credits: (json['credits'] as num?)?.toDouble() ?? 0.0,
      costUsd: (json['cost_usd'] as num?)?.toDouble() ?? 0.0,
      activeMs: json['active_ms'] ?? 0,
    );
  }
}

class AppSummary {
  final String app;
  final int events;
  final int inputTokens;
  final int outputTokens;
  final int reasoningTokens;
  final int cacheReadTokens;
  final int cacheWriteTokens;
  final double credits;
  final double costUsd;

  AppSummary({
    required this.app,
    required this.events,
    required this.inputTokens,
    required this.outputTokens,
    required this.reasoningTokens,
    required this.cacheReadTokens,
    required this.cacheWriteTokens,
    required this.credits,
    required this.costUsd,
  });

  int get totalTokens => inputTokens + outputTokens + cacheReadTokens;

  factory AppSummary.fromJson(Map<String, dynamic> json) {
    return AppSummary(
      app: json['app'] ?? '',
      events: json['events'] ?? 0,
      inputTokens: json['input_tokens'] ?? 0,
      outputTokens: json['output_tokens'] ?? 0,
      reasoningTokens: json['reasoning_tokens'] ?? 0,
      cacheReadTokens: json['cache_read_tokens'] ?? 0,
      cacheWriteTokens: json['cache_write_tokens'] ?? 0,
      credits: (json['credits'] as num?)?.toDouble() ?? 0.0,
      costUsd: (json['cost_usd'] as num?)?.toDouble() ?? 0.0,
    );
  }
}

class ShareRow {
  final String name;
  final int events;
  final int tokens;
  final double costUsd;

  ShareRow({
    required this.name,
    required this.events,
    required this.tokens,
    required this.costUsd,
  });

  factory ShareRow.fromJson(Map<String, dynamic> json) {
    return ShareRow(
      name: json['name'] ?? '',
      events: json['events'] ?? 0,
      tokens: json['tokens'] ?? 0,
      costUsd: (json['cost_usd'] as num?)?.toDouble() ?? 0.0,
    );
  }
}

class TrendBucket {
  final String date;
  final int events;
  final int tokens;
  final double costUsd;
  final List<MapEntry<String, int>> top;

  TrendBucket({
    required this.date,
    required this.events,
    required this.tokens,
    required this.costUsd,
    required this.top,
  });

  factory TrendBucket.fromJson(Map<String, dynamic> json) {
    var rawTop = json['top'] as List? ?? [];
    List<MapEntry<String, int>> parsedTop = [];
    for (var item in rawTop) {
      if (item is List && item.length >= 2) {
        parsedTop.add(MapEntry(item[0].toString(), (item[1] as num).toInt()));
      }
    }
    return TrendBucket(
      date: json['date'] ?? '',
      events: json['events'] ?? 0,
      tokens: json['tokens'] ?? 0,
      costUsd: (json['cost_usd'] as num?)?.toDouble() ?? 0.0,
      top: parsedTop,
    );
  }
}

class ActivityDay {
  final String date;
  final int events;
  final int tokens;
  final double costUsd;

  ActivityDay({
    required this.date,
    required this.events,
    required this.tokens,
    required this.costUsd,
  });

  factory ActivityDay.fromJson(Map<String, dynamic> json) {
    return ActivityDay(
      date: json['date'] ?? '',
      events: json['events'] ?? 0,
      tokens: json['tokens'] ?? 0,
      costUsd: (json['cost_usd'] as num?)?.toDouble() ?? 0.0,
    );
  }
}

class QuotaRow {
  final String app;
  final String? account;
  final int capturedAt;
  final String windowKind;
  final double? used;
  final double? limitValue;
  final double? usedPercent;
  final int? resetsAt;

  QuotaRow({
    required this.app,
    this.account,
    required this.capturedAt,
    required this.windowKind,
    this.used,
    this.limitValue,
    this.usedPercent,
    this.resetsAt,
  });

  factory QuotaRow.fromJson(Map<String, dynamic> json) {
    return QuotaRow(
      app: json['app'] ?? '',
      account: json['account'],
      capturedAt: json['captured_at'] ?? 0,
      windowKind: json['window_kind'] ?? '',
      used: (json['used'] as num?)?.toDouble(),
      limitValue: (json['limit_value'] as num?)?.toDouble(),
      usedPercent: (json['used_percent'] as num?)?.toDouble(),
      resetsAt: json['resets_at'],
    );
  }
}

class QuotaGroup {
  final String app;
  final String display;
  final double? worstPct;
  final List<QuotaRow> rows;

  QuotaGroup({
    required this.app,
    required this.display,
    this.worstPct,
    required this.rows,
  });

  factory QuotaGroup.fromJson(Map<String, dynamic> json) {
    var rowsJson = (json['rows'] as List? ?? [])
        .map((r) => QuotaRow.fromJson(r as Map<String, dynamic>))
        .toList();
    return QuotaGroup(
      app: json['app'] ?? '',
      display: json['display'] ?? '',
      worstPct: (json['worst_pct'] as num?)?.toDouble(),
      rows: rowsJson,
    );
  }
}

class OverviewData {
  final Totals today;
  final Totals span;
  final Totals all;
  final String rangeType;
  final List<AppSummary> byApp;
  final List<ShareRow> byModel;
  final List<TrendBucket> daily;
  final List<TrendBucket> daily30d;
  final List<ActivityDay> activity;
  final List<String> apps;
  final List<String> models;
  final List<QuotaRow> quotas;
  final List<QuotaGroup> quotaGroups;
  final String tzOffset;
  final TokenRateOverview? tokenRates;

  OverviewData({
    required this.today,
    required this.span,
    required this.all,
    required this.rangeType,
    required this.byApp,
    required this.byModel,
    required this.daily,
    this.daily30d = const [],
    this.activity = const [],
    required this.apps,
    required this.models,
    required this.quotas,
    required this.quotaGroups,
    required this.tzOffset,
    this.tokenRates,
  });

  factory OverviewData.fromJson(Map<String, dynamic> json) {
    return OverviewData(
      today: Totals.fromJson(json['today'] ?? {}),
      span: Totals.fromJson(json['span'] ?? {}),
      all: Totals.fromJson(json['all'] ?? {}),
      rangeType: json['range']?['type'] ?? 'week',
      byApp: (json['by_app'] as List? ?? [])
          .map((a) => AppSummary.fromJson(a as Map<String, dynamic>))
          .toList(),
      byModel: (json['by_model'] as List? ?? [])
          .map((m) => ShareRow.fromJson(m as Map<String, dynamic>))
          .toList(),
      daily: (json['daily'] as List? ?? [])
          .map((d) => TrendBucket.fromJson(d as Map<String, dynamic>))
          .toList(),
      daily30d: (json['daily_30d'] as List? ?? [])
          .map((d) => TrendBucket.fromJson(d as Map<String, dynamic>))
          .toList(),
      activity: (json['activity'] as List? ?? [])
          .map((a) => ActivityDay.fromJson(a as Map<String, dynamic>))
          .toList(),
      apps: (json['apps'] as List? ?? []).map((e) => e.toString()).toList(),
      models: (json['models'] as List? ?? []).map((e) => e.toString()).toList(),
      quotas: (json['quotas'] as List? ?? [])
          .map((q) => QuotaRow.fromJson(q as Map<String, dynamic>))
          .toList(),
      quotaGroups: (json['quota_groups'] as List? ?? [])
          .map((g) => QuotaGroup.fromJson(g as Map<String, dynamic>))
          .toList(),
      tzOffset: json['tz_offset'] ?? '+00:00',
      tokenRates: json['token_rates'] != null
          ? TokenRateOverview.fromJson(json['token_rates'] as Map<String, dynamic>)
          : null,
    );
  }
}

class RateMetric {
  final String window;
  final int windowSecs;
  final int startMs;
  final int endMs;
  final int events;
  final int totalTokens;
  final int inputTokens;
  final int outputTokens;
  final int reasoningTokens;
  final int cacheReadTokens;
  final int cacheWriteTokens;
  final double costUsd;
  final double tokensPerMin;
  final double tokensPerSec;
  final double inputTokensPerMin;
  final double outputTokensPerMin;
  final double costPerHour;
  final double requestsPerMin;

  RateMetric({
    required this.window,
    required this.windowSecs,
    required this.startMs,
    required this.endMs,
    required this.events,
    required this.totalTokens,
    required this.inputTokens,
    required this.outputTokens,
    required this.reasoningTokens,
    required this.cacheReadTokens,
    required this.cacheWriteTokens,
    required this.costUsd,
    required this.tokensPerMin,
    required this.tokensPerSec,
    required this.inputTokensPerMin,
    required this.outputTokensPerMin,
    required this.costPerHour,
    required this.requestsPerMin,
  });

  factory RateMetric.fromJson(Map<String, dynamic> json) {
    return RateMetric(
      window: json['window'] ?? '',
      windowSecs: (json['window_secs'] as num?)?.toInt() ?? 0,
      startMs: (json['start_ms'] as num?)?.toInt() ?? 0,
      endMs: (json['end_ms'] as num?)?.toInt() ?? 0,
      events: (json['events'] as num?)?.toInt() ?? 0,
      totalTokens: (json['total_tokens'] as num?)?.toInt() ?? 0,
      inputTokens: (json['input_tokens'] as num?)?.toInt() ?? 0,
      outputTokens: (json['output_tokens'] as num?)?.toInt() ?? 0,
      reasoningTokens: (json['reasoning_tokens'] as num?)?.toInt() ?? 0,
      cacheReadTokens: (json['cache_read_tokens'] as num?)?.toInt() ?? 0,
      cacheWriteTokens: (json['cache_write_tokens'] as num?)?.toInt() ?? 0,
      costUsd: (json['cost_usd'] as num?)?.toDouble() ?? 0.0,
      tokensPerMin: (json['tokens_per_min'] as num?)?.toDouble() ?? 0.0,
      tokensPerSec: (json['tokens_per_sec'] as num?)?.toDouble() ?? 0.0,
      inputTokensPerMin: (json['input_tokens_per_min'] as num?)?.toDouble() ?? 0.0,
      outputTokensPerMin: (json['output_tokens_per_min'] as num?)?.toDouble() ?? 0.0,
      costPerHour: (json['cost_per_hour'] as num?)?.toDouble() ?? 0.0,
      requestsPerMin: (json['requests_per_min'] as num?)?.toDouble() ?? 0.0,
    );
  }
}

class ModelRateItem {
  final String model;
  final String app;
  final int events;
  final int totalTokens;
  final double tokensPerMin;
  final double tokensPerSec;
  final double costUsd;
  final double percentage;

  ModelRateItem({
    required this.model,
    required this.app,
    required this.events,
    required this.totalTokens,
    required this.tokensPerMin,
    required this.tokensPerSec,
    required this.costUsd,
    required this.percentage,
  });

  factory ModelRateItem.fromJson(Map<String, dynamic> json) {
    return ModelRateItem(
      model: json['model'] ?? '',
      app: json['app'] ?? '',
      events: (json['events'] as num?)?.toInt() ?? 0,
      totalTokens: (json['total_tokens'] as num?)?.toInt() ?? 0,
      tokensPerMin: (json['tokens_per_min'] as num?)?.toDouble() ?? 0.0,
      tokensPerSec: (json['tokens_per_sec'] as num?)?.toDouble() ?? 0.0,
      costUsd: (json['cost_usd'] as num?)?.toDouble() ?? 0.0,
      percentage: (json['percentage'] as num?)?.toDouble() ?? 0.0,
    );
  }
}

class RateTimeSeriesBucket {
  final int timestampMs;
  final String label;
  final int totalTokens;
  final int inputTokens;
  final int outputTokens;
  final int cacheReadTokens;
  final double tokensPerSec;
  final double tokensPerMin;
  final int events;
  final double costUsd;

  RateTimeSeriesBucket({
    required this.timestampMs,
    required this.label,
    required this.totalTokens,
    required this.inputTokens,
    required this.outputTokens,
    required this.cacheReadTokens,
    required this.tokensPerSec,
    required this.tokensPerMin,
    required this.events,
    required this.costUsd,
  });

  factory RateTimeSeriesBucket.fromJson(Map<String, dynamic> json) {
    return RateTimeSeriesBucket(
      timestampMs: (json['timestamp_ms'] as num?)?.toInt() ?? 0,
      label: json['label'] ?? '',
      totalTokens: (json['total_tokens'] as num?)?.toInt() ?? 0,
      inputTokens: (json['input_tokens'] as num?)?.toInt() ?? 0,
      outputTokens: (json['output_tokens'] as num?)?.toInt() ?? 0,
      cacheReadTokens: (json['cache_read_tokens'] as num?)?.toInt() ?? 0,
      tokensPerSec: (json['tokens_per_sec'] as num?)?.toDouble() ?? 0.0,
      tokensPerMin: (json['tokens_per_min'] as num?)?.toDouble() ?? 0.0,
      events: (json['events'] as num?)?.toInt() ?? 0,
      costUsd: (json['cost_usd'] as num?)?.toDouble() ?? 0.0,
    );
  }
}

class TokenRateOverview {
  final int anchorMs;
  final int? latestEventMs;
  final bool isActive;
  final RateMetric m1;
  final RateMetric m5;
  final RateMetric m15;
  final RateMetric h1;
  final RateMetric h24;
  final RateMetric peak1mIn1h;
  final RateMetric peak1mIn24h;
  final List<RateTimeSeriesBucket> timeline1h;
  final List<ModelRateItem> topModels1h;
  final List<ModelRateItem> topApps1h;

  TokenRateOverview({
    required this.anchorMs,
    this.latestEventMs,
    required this.isActive,
    required this.m1,
    required this.m5,
    required this.m15,
    required this.h1,
    required this.h24,
    required this.peak1mIn1h,
    required this.peak1mIn24h,
    required this.timeline1h,
    required this.topModels1h,
    required this.topApps1h,
  });

  factory TokenRateOverview.fromJson(Map<String, dynamic> json) {
    return TokenRateOverview(
      anchorMs: (json['anchor_ms'] as num?)?.toInt() ?? 0,
      latestEventMs: (json['latest_event_ms'] as num?)?.toInt(),
      isActive: json['is_active'] ?? false,
      m1: RateMetric.fromJson(json['m1'] as Map<String, dynamic>? ?? {}),
      m5: RateMetric.fromJson(json['m5'] as Map<String, dynamic>? ?? {}),
      m15: RateMetric.fromJson(json['m15'] as Map<String, dynamic>? ?? {}),
      h1: RateMetric.fromJson(json['h1'] as Map<String, dynamic>? ?? {}),
      h24: RateMetric.fromJson(json['h24'] as Map<String, dynamic>? ?? {}),
      peak1mIn1h: RateMetric.fromJson(json['peak_1m_in_1h'] as Map<String, dynamic>? ?? {}),
      peak1mIn24h: RateMetric.fromJson(json['peak_1m_in_24h'] as Map<String, dynamic>? ?? {}),
      timeline1h: (json['timeline_1h'] as List? ?? [])
          .map((b) => RateTimeSeriesBucket.fromJson(b as Map<String, dynamic>))
          .toList(),
      topModels1h: (json['top_models_1h'] as List? ?? [])
          .map((m) => ModelRateItem.fromJson(m as Map<String, dynamic>))
          .toList(),
      topApps1h: (json['top_apps_1h'] as List? ?? [])
          .map((a) => ModelRateItem.fromJson(a as Map<String, dynamic>))
          .toList(),
    );
  }
}

class EventRow {
  final String app;
  final String? model;
  final String? pricingModel;
  final String? project;
  final String? sessionId;
  final int? tsStart;
  final int inputTokens;
  final int outputTokens;
  final int reasoningTokens;
  final int cacheReadTokens;
  final int cacheWriteTokens;
  final double? credits;
  final double? costUsd;
  final String? costSource;
  final int? durationMs;
  final String? rawRef;

  EventRow({
    required this.app,
    this.model,
    this.pricingModel,
    this.project,
    this.sessionId,
    this.tsStart,
    required this.inputTokens,
    required this.outputTokens,
    required this.reasoningTokens,
    required this.cacheReadTokens,
    required this.cacheWriteTokens,
    this.credits,
    this.costUsd,
    this.costSource,
    this.durationMs,
    this.rawRef,
  });

  int get totalTokens => inputTokens + outputTokens + cacheReadTokens;

  factory EventRow.fromJson(Map<String, dynamic> json) {
    return EventRow(
      app: json['app'] ?? '',
      model: json['model'],
      pricingModel: json['pricing_model'],
      project: json['project'],
      sessionId: json['session_id'],
      tsStart: json['ts_start'],
      inputTokens: json['input_tokens'] ?? 0,
      outputTokens: json['output_tokens'] ?? 0,
      reasoningTokens: json['reasoning_tokens'] ?? 0,
      cacheReadTokens: json['cache_read_tokens'] ?? 0,
      cacheWriteTokens: json['cache_write_tokens'] ?? 0,
      credits: (json['credits'] as num?)?.toDouble(),
      costUsd: (json['cost_usd'] as num?)?.toDouble(),
      costSource: json['cost_source'],
      durationMs: json['duration_ms'],
      rawRef: json['raw_ref'],
    );
  }
}

class DetailData {
  final List<EventRow> rows;
  final int totalEvents;

  DetailData({required this.rows, required this.totalEvents});

  factory DetailData.fromJson(Map<String, dynamic> json) {
    return DetailData(
      rows: (json['rows'] as List? ?? [])
          .map((r) => EventRow.fromJson(r as Map<String, dynamic>))
          .toList(),
      totalEvents: json['total_events'] ?? 0,
    );
  }
}

class SourceHealth {
  final String source;
  final bool enabled;
  final int? lastSyncedAt;
  final String? lastError;
  final int filesSeen;
  final int rowsIngested;
  final int cursors;

  SourceHealth({
    required this.source,
    required this.enabled,
    this.lastSyncedAt,
    this.lastError,
    required this.filesSeen,
    required this.rowsIngested,
    required this.cursors,
  });

  factory SourceHealth.fromJson(Map<String, dynamic> json) {
    return SourceHealth(
      source: json['source'] ?? '',
      enabled: json['enabled'] ?? true,
      lastSyncedAt: json['last_synced_at'],
      lastError: json['last_error'],
      filesSeen: json['files_seen'] ?? 0,
      rowsIngested: json['rows_ingested'] ?? 0,
      cursors: json['cursors'] ?? 0,
    );
  }
}

class PriceRow {
  final String model;
  final double input;
  final double output;
  final double cacheRead;
  final double cacheWrite;
  final String source;

  PriceRow({
    required this.model,
    required this.input,
    required this.output,
    required this.cacheRead,
    required this.cacheWrite,
    required this.source,
  });

  factory PriceRow.fromJson(Map<String, dynamic> json) {
    return PriceRow(
      model: json['model'] ?? '',
      input: (json['input'] as num?)?.toDouble() ?? 0.0,
      output: (json['output'] as num?)?.toDouble() ?? 0.0,
      cacheRead: (json['cache_read'] as num?)?.toDouble() ?? 0.0,
      cacheWrite: (json['cache_write'] as num?)?.toDouble() ?? 0.0,
      source: json['source'] ?? '',
    );
  }
}
