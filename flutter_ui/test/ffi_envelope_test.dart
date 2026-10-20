import 'package:flutter_test/flutter_test.dart';

import 'package:globaltokentracker_ui/core/ffi_envelope.dart';

void main() {
  test('passes successful map and list payloads through', () {
    expect(decodeFfiEnvelope('{"span":{"events":2}}'), isA<Map>());
    expect(decodeFfiEnvelope('[{"source":"claude"}]'), isA<List>());
    expect(decodeFfiEnvelope('{"ok":true,"updated":3}')['updated'], 3);
  });

  test('throws the backend error instead of returning an empty payload', () {
    expect(
      () => decodeFfiEnvelope('{"ok":false,"error":"database is locked"}'),
      throwsA(
        isA<Exception>().having(
          (e) => e.toString(),
          'message',
          contains('database is locked'),
        ),
      ),
    );
  });

  test('falls back to a readable message for a blank error', () {
    expect(
      () => decodeFfiEnvelope('{"ok":false}'),
      throwsA(
        isA<Exception>().having(
          (e) => e.toString(),
          'message',
          contains('Unknown FFI error'),
        ),
      ),
    );
  });
}
