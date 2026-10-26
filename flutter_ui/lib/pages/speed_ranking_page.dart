import 'package:flutter/material.dart';
import '../core/ffi_bridge.dart';
import '../widgets/statistics_range_picker.dart';

/// Historical rankings are separate from rolling live velocity windows.
class SpeedRankingPage extends StatefulWidget {
  final List<String> apps;
  final List<String> models;
  final Future<Map<String, dynamic>> Function(Map<String, dynamic>)? loader;
  const SpeedRankingPage(
      {super.key, this.apps = const [], this.models = const [], this.loader});
  @override
  State<SpeedRankingPage> createState() => _SpeedRankingPageState();
}

class _SpeedRankingPageState extends State<SpeedRankingPage> {
  static StatisticsWindow _window = const StatisticsWindow('week');
  static String _group = 'model', _metric = 'output';
  String? _app, _model;
  int _page = 0, _total = 0, _generation = 0;
  bool _loading = true;
  String? _error;
  List<Map<String, dynamic>> _rows = [];
  static const _metrics = {
    'output': '输出速度',
    'total': '总吞吐',
    'input': '输入速度',
    'prefill': '预填速度',
    'decode': '解码速度'
  };

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load({bool reset = false}) async {
    if (reset) _page = 0;
    final generation = ++_generation;
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final request = <String, dynamic>{
        'range_key': _window.key,
        'custom_start_ms': _window.startMs,
        'custom_end_ms': _window.endMs,
        'group_by': _group,
        'metric': _metric,
        'apps': _app == null ? null : [_app],
        'models': _model == null ? null : [_model],
        'page': _page,
        'page_size': 25,
      };
      final result = await (widget.loader?.call(request) ??
          FfiBridge.instance.queryJson('gtt_get_speed_ranking', request));
      if (!mounted || generation != _generation) return;
      setState(() {
        _rows = (result['rows'] as List)
            .map((r) => Map<String, dynamic>.from(r))
            .toList();
        _total = result['total'] as int;
        _page = result['page'] as int;
        _loading = false;
      });
    } catch (e) {
      if (mounted && generation == _generation) {
        setState(() {
          _error = e.toString();
          _loading = false;
        });
      }
    }
  }

  String _rate(dynamic value) =>
      value == null ? '—' : '${(value as num).toStringAsFixed(1)} tok/s';

  @override
  Widget build(BuildContext context) => Scaffold(
        appBar: AppBar(title: const Text('历史速度排行榜'), actions: [
          IconButton(
              onPressed: () => _load(),
              icon: const Icon(Icons.refresh),
              tooltip: '刷新')
        ]),
        body: Padding(
            padding: const EdgeInsets.all(24),
            child:
                Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              StatisticsRangePicker(
                  value: _window,
                  onChanged: (w) {
                    _window = w;
                    _load(reset: true);
                  }),
              const SizedBox(height: 12),
              Wrap(
                  spacing: 16,
                  runSpacing: 8,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                    SegmentedButton<String>(
                        segments: const [
                          ButtonSegment(value: 'model', label: Text('按模型')),
                          ButtonSegment(value: 'app', label: Text('按工具'))
                        ],
                        selected: {
                          _group
                        },
                        onSelectionChanged: (s) {
                          _group = s.first;
                          _load(reset: true);
                        }),
                    DropdownButton<String>(
                        value: _metric,
                        items: [
                          for (final e in _metrics.entries)
                            DropdownMenuItem(
                                value: e.key, child: Text('按${e.value}排序'))
                        ],
                        onChanged: (m) {
                          _metric = m!;
                          _load(reset: true);
                        }),
                    DropdownButton<String>(
                        value: _app,
                        hint: const Text('全部工具'),
                        items: [
                          const DropdownMenuItem<String>(
                              value: null, child: Text('全部工具')),
                          for (final a in widget.apps)
                            DropdownMenuItem(value: a, child: Text(a))
                        ],
                        onChanged: (v) {
                          _app = v;
                          _load(reset: true);
                        }),
                    SizedBox(
                        width: 220,
                        child: DropdownButton<String>(
                            isExpanded: true,
                            value: _model,
                            hint: const Text('全部模型'),
                            items: [
                              const DropdownMenuItem<String>(
                                  value: null, child: Text('全部模型')),
                              for (final m in widget.models)
                                DropdownMenuItem(
                                    value: m,
                                    child: Text(m,
                                        overflow: TextOverflow.ellipsis))
                            ],
                            onChanged: (v) {
                              _model = v;
                              _load(reset: true);
                            })),
                  ]),
              const SizedBox(height: 12),
              const Text('速率按有效 Token / 有效时长加权；无计时显示 —，估算时长单独标注。'),
              if (_loading) const LinearProgressIndicator(),
              if (_error != null)
                Padding(
                    padding: const EdgeInsets.all(12),
                    child: Text(_error!,
                        style: TextStyle(
                            color: Theme.of(context).colorScheme.error))),
              const SizedBox(height: 8),
              Expanded(
                  child: _rows.isEmpty && !_loading
                      ? const Center(child: Text('所选范围暂无记录'))
                      : ListView.builder(
                          itemCount: _rows.length,
                          itemBuilder: (context, i) {
                            final row = _rows[i], speed = row['speed'] as Map;
                            return Card(
                                child: Padding(
                                    padding: const EdgeInsets.all(16),
                                    child: Column(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.start,
                                        children: [
                                          Text(
                                              '${_page * 25 + i + 1}. ${row['name']}',
                                              style: Theme.of(context)
                                                  .textTheme
                                                  .titleMedium),
                                          const SizedBox(height: 8),
                                          Wrap(
                                              spacing: 24,
                                              runSpacing: 8,
                                              children: [
                                                for (final e
                                                    in _metrics.entries)
                                                  Text(
                                                      '${e.value}  ${_rate(row['${e.key}_tps'])}',
                                                      style: TextStyle(
                                                          fontWeight: _metric ==
                                                                  e.key
                                                              ? FontWeight.bold
                                                              : FontWeight
                                                                  .normal))
                                              ]),
                                          const SizedBox(height: 8),
                                          Text(
                                              '${row['events']} 次记录 · ${speed['timed_events']} 次有效计时 · TTFT ${row['avg_ttft_ms'] == null ? '—' : '${(row['avg_ttft_ms'] as num).toStringAsFixed(0)} ms'} · 估算时长 ${((speed['speed_est_ms'] as num) / 1000).toStringAsFixed(1)} s'),
                                        ])));
                          })),
              Row(mainAxisAlignment: MainAxisAlignment.end, children: [
                Text(
                    '共 $_total 组 · 第 ${_page + 1} / ${((_total + 24) ~/ 25).clamp(1, 1000000)} 页'),
                IconButton(
                    onPressed: _loading || _page == 0
                        ? null
                        : () {
                            _page--;
                            _load();
                          },
                    icon: const Icon(Icons.chevron_left)),
                IconButton(
                    onPressed: _loading || (_page + 1) * 25 >= _total
                        ? null
                        : () {
                            _page++;
                            _load();
                          },
                    icon: const Icon(Icons.chevron_right))
              ]),
            ])),
      );
}
