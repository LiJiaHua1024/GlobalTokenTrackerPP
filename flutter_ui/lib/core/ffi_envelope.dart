import 'dart:convert';

/// Decodes one JSON string returned by the Rust FFI.
///
/// Successful calls return their payload directly; failures return
/// `{"ok":false,"error":"…"}`. Every caller must go through this helper so a
/// backend error can never be mistaken for an empty dashboard or a successful
/// scan.
dynamic decodeFfiEnvelope(String raw) {
  final json = jsonDecode(raw);
  if (json is Map && json['ok'] == false) {
    final error = json['error'];
    throw Exception(
      error is String && error.isNotEmpty ? error : 'Unknown FFI error',
    );
  }
  return json;
}
