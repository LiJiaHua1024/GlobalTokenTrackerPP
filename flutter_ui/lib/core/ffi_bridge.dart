import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'package:ffi/ffi.dart';
import 'models.dart';

// Native function typedefs
typedef _gtt_default_db_path_c = ffi.Pointer<Utf8> Function();
typedef _gtt_default_db_path_dart = ffi.Pointer<Utf8> Function();

typedef _gtt_free_string_c = ffi.Void Function(ffi.Pointer<Utf8> ptr);
typedef _gtt_free_string_dart = void Function(ffi.Pointer<Utf8> ptr);

typedef _gtt_engine_open_c = ffi.Pointer<ffi.Void> Function(ffi.Pointer<Utf8> dbPath);
typedef _gtt_engine_open_dart = ffi.Pointer<ffi.Void> Function(ffi.Pointer<Utf8> dbPath);

typedef _gtt_engine_close_c = ffi.Void Function(ffi.Pointer<ffi.Void> ctx);
typedef _gtt_engine_close_dart = void Function(ffi.Pointer<ffi.Void> ctx);

typedef _gtt_engine_scan_c = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);
typedef _gtt_engine_scan_dart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);

typedef _gtt_get_overview_c = ffi.Pointer<Utf8> Function(
  ffi.Pointer<ffi.Void> ctx,
  ffi.Pointer<Utf8> rangeKey,
  ffi.Int64 customStartMs,
  ffi.Int64 customEndMs,
  ffi.Pointer<Utf8> filterAppsJson,
  ffi.Pointer<Utf8> filterModelsJson,
);
typedef _gtt_get_overview_dart = ffi.Pointer<Utf8> Function(
  ffi.Pointer<ffi.Void> ctx,
  ffi.Pointer<Utf8> rangeKey,
  int customStartMs,
  int customEndMs,
  ffi.Pointer<Utf8> filterAppsJson,
  ffi.Pointer<Utf8> filterModelsJson,
);

typedef _gtt_get_details_c = ffi.Pointer<Utf8> Function(
  ffi.Pointer<ffi.Void> ctx,
  ffi.Int64 page,
  ffi.Int64 pageSize,
  ffi.Pointer<Utf8> filterAppsJson,
  ffi.Pointer<Utf8> filterModelsJson,
);
typedef _gtt_get_details_dart = ffi.Pointer<Utf8> Function(
  ffi.Pointer<ffi.Void> ctx,
  int page,
  int pageSize,
  ffi.Pointer<Utf8> filterAppsJson,
  ffi.Pointer<Utf8> filterModelsJson,
);

typedef _gtt_get_sources_c = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);
typedef _gtt_get_sources_dart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);

typedef _gtt_get_prices_c = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx, ffi.Size limit);
typedef _gtt_get_prices_dart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx, int limit);

typedef _gtt_update_prices_c = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);
typedef _gtt_update_prices_dart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);

typedef _gtt_poll_quotas_c = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);
typedef _gtt_poll_quotas_dart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);

class FfiBridge {
  static FfiBridge? _instance;
  late ffi.DynamicLibrary _dylib;

  late _gtt_default_db_path_dart _defaultDbPath;
  late _gtt_free_string_dart _freeString;
  late _gtt_engine_open_dart _engineOpen;
  late _gtt_engine_close_dart _engineClose;
  late _gtt_engine_scan_dart _engineScan;
  late _gtt_get_overview_dart _getOverview;
  late _gtt_get_details_dart _getDetails;
  late _gtt_get_sources_dart _getSources;
  late _gtt_get_prices_dart _getPrices;
  late _gtt_update_prices_dart _updatePrices;
  late _gtt_poll_quotas_dart _pollQuotas;

  ffi.Pointer<ffi.Void>? _context;

  static FfiBridge get instance => _instance ??= FfiBridge._();

  FfiBridge._() {
    _loadDylib();
    _bindFunctions();
  }

  void _loadDylib() {
    final possiblePaths = [
      'globaltokentracker_ffi.dll',
      r'F:\Agentic Project\GlobalTokenTracker\target\debug\globaltokentracker_ffi.dll',
      r'F:\Agentic Project\GlobalTokenTracker\target\release\globaltokentracker_ffi.dll',
      r'target\debug\globaltokentracker_ffi.dll',
      r'target\release\globaltokentracker_ffi.dll',
      r'..\target\debug\globaltokentracker_ffi.dll',
      r'..\target\release\globaltokentracker_ffi.dll',
    ];

    for (final p in possiblePaths) {
      if (File(p).existsSync()) {
        _dylib = ffi.DynamicLibrary.open(p);
        return;
      }
    }

    try {
      _dylib = ffi.DynamicLibrary.open('globaltokentracker_ffi.dll');
    } catch (_) {
      _dylib = ffi.DynamicLibrary.open(
        r'F:\Agentic Project\GlobalTokenTracker\target\debug\globaltokentracker_ffi.dll',
      );
    }
  }

  void _bindFunctions() {
    _defaultDbPath = _dylib
        .lookup<ffi.NativeFunction<_gtt_default_db_path_c>>('gtt_default_db_path')
        .asFunction();
    _freeString = _dylib
        .lookup<ffi.NativeFunction<_gtt_free_string_c>>('gtt_free_string')
        .asFunction();
    _engineOpen = _dylib
        .lookup<ffi.NativeFunction<_gtt_engine_open_c>>('gtt_engine_open')
        .asFunction();
    _engineClose = _dylib
        .lookup<ffi.NativeFunction<_gtt_engine_close_c>>('gtt_engine_close')
        .asFunction();
    _engineScan = _dylib
        .lookup<ffi.NativeFunction<_gtt_engine_scan_c>>('gtt_engine_scan')
        .asFunction();
    _getOverview = _dylib
        .lookup<ffi.NativeFunction<_gtt_get_overview_c>>('gtt_get_overview')
        .asFunction();
    _getDetails = _dylib
        .lookup<ffi.NativeFunction<_gtt_get_details_c>>('gtt_get_details')
        .asFunction();
    _getSources = _dylib
        .lookup<ffi.NativeFunction<_gtt_get_sources_c>>('gtt_get_sources')
        .asFunction();
    _getPrices = _dylib
        .lookup<ffi.NativeFunction<_gtt_get_prices_c>>('gtt_get_prices')
        .asFunction();
    _updatePrices = _dylib
        .lookup<ffi.NativeFunction<_gtt_update_prices_c>>('gtt_update_prices')
        .asFunction();
    _pollQuotas = _dylib
        .lookup<ffi.NativeFunction<_gtt_poll_quotas_c>>('gtt_poll_quotas')
        .asFunction();
  }

  String getDefaultDbPath() {
    final ptr = _defaultDbPath();
    final res = ptr.toDartString();
    _freeString(ptr);
    return res;
  }

  bool openEngine({String? dbPath}) {
    if (_context != null) return true;
    final pathPtr = dbPath != null ? dbPath.toNativeUtf8() : ffi.nullptr;
    _context = _engineOpen(pathPtr);
    if (pathPtr != ffi.nullptr) {
      calloc.free(pathPtr);
    }
    return _context != null && _context!.address != 0;
  }

  void closeEngine() {
    if (_context != null && _context!.address != 0) {
      _engineClose(_context!);
      _context = null;
    }
  }

  String _consumeString(ffi.Pointer<Utf8> ptr) {
    if (ptr == ffi.nullptr) return '';
    final res = ptr.toDartString();
    _freeString(ptr);
    return res;
  }

  Future<Map<String, dynamic>> scan() async {
    if (_context == null) throw Exception("Engine not initialized");
    final ptr = _engineScan(_context!);
    final raw = _consumeString(ptr);
    return jsonDecode(raw) as Map<String, dynamic>;
  }

  Future<OverviewData> getOverview({
    String rangeKey = 'week',
    int customStartMs = 0,
    int customEndMs = 0,
    List<String>? filterApps,
    List<String>? filterModels,
  }) async {
    if (_context == null) throw Exception("Engine not initialized");

    final rangePtr = rangeKey.toNativeUtf8();
    final appsPtr = filterApps != null && filterApps.isNotEmpty
        ? jsonEncode(filterApps).toNativeUtf8()
        : ffi.nullptr;
    final modelsPtr = filterModels != null && filterModels.isNotEmpty
        ? jsonEncode(filterModels).toNativeUtf8()
        : ffi.nullptr;

    final ptr = _getOverview(
      _context!,
      rangePtr,
      customStartMs,
      customEndMs,
      appsPtr,
      modelsPtr,
    );

    calloc.free(rangePtr);
    if (appsPtr != ffi.nullptr) calloc.free(appsPtr);
    if (modelsPtr != ffi.nullptr) calloc.free(modelsPtr);

    final raw = _consumeString(ptr);
    final json = jsonDecode(raw) as Map<String, dynamic>;
    return OverviewData.fromJson(json);
  }

  Future<DetailData> getDetails({
    int page = 0,
    int pageSize = 50,
    List<String>? filterApps,
    List<String>? filterModels,
  }) async {
    if (_context == null) throw Exception("Engine not initialized");

    final appsPtr = filterApps != null && filterApps.isNotEmpty
        ? jsonEncode(filterApps).toNativeUtf8()
        : ffi.nullptr;
    final modelsPtr = filterModels != null && filterModels.isNotEmpty
        ? jsonEncode(filterModels).toNativeUtf8()
        : ffi.nullptr;

    final ptr = _getDetails(_context!, page, pageSize, appsPtr, modelsPtr);

    if (appsPtr != ffi.nullptr) calloc.free(appsPtr);
    if (modelsPtr != ffi.nullptr) calloc.free(modelsPtr);

    final raw = _consumeString(ptr);
    final json = jsonDecode(raw) as Map<String, dynamic>;
    return DetailData.fromJson(json);
  }

  Future<List<SourceHealth>> getSources() async {
    if (_context == null) throw Exception("Engine not initialized");
    final ptr = _getSources(_context!);
    final raw = _consumeString(ptr);
    final list = jsonDecode(raw) as List;
    return list.map((e) => SourceHealth.fromJson(e as Map<String, dynamic>)).toList();
  }

  Future<List<PriceRow>> getPrices({int limit = 5000}) async {
    if (_context == null) throw Exception("Engine not initialized");
    final ptr = _getPrices(_context!, limit);
    final raw = _consumeString(ptr);
    final list = jsonDecode(raw) as List;
    return list.map((e) => PriceRow.fromJson(e as Map<String, dynamic>)).toList();
  }

  Future<Map<String, dynamic>> updatePrices() async {
    if (_context == null) throw Exception("Engine not initialized");
    final ptr = _updatePrices(_context!);
    final raw = _consumeString(ptr);
    return jsonDecode(raw) as Map<String, dynamic>;
  }

  Future<Map<String, dynamic>> pollQuotas() async {
    if (_context == null) throw Exception("Engine not initialized");
    final ptr = _pollQuotas(_context!);
    final raw = _consumeString(ptr);
    return jsonDecode(raw) as Map<String, dynamic>;
  }
}
