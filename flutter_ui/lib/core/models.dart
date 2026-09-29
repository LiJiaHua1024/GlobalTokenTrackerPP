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
  final List<String> apps;
  final List<String> models;
  final List<QuotaRow> quotas;
  final List<QuotaGroup> quotaGroups;
  final String tzOffset;

  OverviewData({
    required this.today,
    required this.span,
    required this.all,
    required this.rangeType,
    required this.byApp,
    required this.byModel,
    required this.daily,
    required this.apps,
    required this.models,
    required this.quotas,
    required this.quotaGroups,
    required this.tzOffset,
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
      apps: (json['apps'] as List? ?? []).map((e) => e.toString()).toList(),
      models: (json['models'] as List? ?? []).map((e) => e.toString()).toList(),
      quotas: (json['quotas'] as List? ?? [])
          .map((q) => QuotaRow.fromJson(q as Map<String, dynamic>))
          .toList(),
      quotaGroups: (json['quota_groups'] as List? ?? [])
          .map((g) => QuotaGroup.fromJson(g as Map<String, dynamic>))
          .toList(),
      tzOffset: json['tz_offset'] ?? '+00:00',
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
