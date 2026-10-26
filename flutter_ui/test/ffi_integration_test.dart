import 'package:flutter_test/flutter_test.dart';
import 'dart:io';
import 'package:globaltokentracker_ui/core/ffi_bridge.dart';

void main() {
  test('display currency persists through the native file-backed ledger', () async {
    final dir = await Directory.systemTemp.createTemp('gtt-fx-');
    final bridge = FfiBridge.instance;
    try {
      bridge.openEngine(dbPath: '${dir.path}/ledger.db');
      final configured = await bridge.queryJson('gtt_fx', {'action': 'set', 'currency': 'CNY'});
      expect(configured['display'], 'CNY');
      bridge.closeEngine();
      bridge.openEngine(dbPath: '${dir.path}/ledger.db');
      final restored = await bridge.queryJson('gtt_fx', {'action': 'status'});
      expect(restored['display'], 'CNY');
      expect(restored['source'], isNull);
    } finally { bridge.closeEngine(); await dir.delete(recursive: true); }
  });
  test('FfiBridge loads native DLL and invokes getTokenRates successfully', () async {
    final bridge = FfiBridge.instance;
    // Open memory database / engine
    bridge.openEngine(dbPath: ':memory:');

    try {
      final rates = await bridge.getTokenRates();
      expect(rates, isNotNull);
      expect(rates.m1, isNotNull);
      expect(rates.h1, isNotNull);
      expect(rates.h24, isNotNull);
      final ranking = await bridge.queryJson('gtt_get_speed_ranking', {'range_key': 'all', 'group_by': 'model', 'metric': 'output'});
      expect(ranking['total'], 0);
      expect(ranking['rows'], isEmpty);
    } finally {
      bridge.closeEngine();
    }
  });
}
