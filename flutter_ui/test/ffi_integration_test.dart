import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/ffi_bridge.dart';

void main() {
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
    } finally {
      bridge.closeEngine();
    }
  });
}
