import 'dart:ffi';

// Keep packaging and runtime checks on the same ABI requirements.
const requiredNativeSymbols = [
  'gtt_default_db_path',
  'gtt_free_string',
  'gtt_engine_open',
  'gtt_engine_close',
  'gtt_engine_scan',
  'gtt_get_overview',
  'gtt_get_token_rates',
  'gtt_get_details',
  'gtt_get_sources',
  'gtt_get_prices',
  'gtt_update_prices',
  'gtt_poll_quotas',
  'gtt_get_speed_ranking',
  'gtt_fx',
];

class NativeRuntimeException implements Exception {
  NativeRuntimeException(this.details);
  final String details;

  @override
  String toString() => '程序文件版本不匹配或安装不完整，请关闭程序后使用完整安装包重新安装。\n$details';
}

DynamicLibrary openVerifiedNativeLibrary(String path) {
  try {
    final library = DynamicLibrary.open(path);
    final missing = requiredNativeSymbols
        .where((symbol) => !library.providesSymbol(symbol))
        .toList();
    if (missing.isNotEmpty) {
      throw NativeRuntimeException('$path\n缺少接口：${missing.join(', ')}');
    }
    return library;
  } on NativeRuntimeException {
    rethrow;
  } catch (error) {
    throw NativeRuntimeException('$path\n$error');
  }
}
