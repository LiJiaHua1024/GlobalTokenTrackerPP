import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:isolate';
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

typedef _gtt_get_prices_c = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx, ffi.Int64 limit);
typedef _gtt_get_prices_dart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx, int limit);

typedef _gtt_update_prices_c = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);
typedef _gtt_update_prices_dart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);

typedef _gtt_poll_quotas_c = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);
typedef _gtt_poll_quotas_dart = ffi.Pointer<Utf8> Function(ffi.Pointer<ffi.Void> ctx);

class FfiBridge {
  static const _autoPriceRefreshInterval = Duration(hours: 12);

  static FfiBridge? _instance;
  late ffi.DynamicLibrary _dylib;
  late String _resolvedDylibPath;

  late _gtt_default_db_path_dart _defaultDbPath;
  late _gtt_free_string_dart _freeString;
  late _gtt_engine_open_dart _engineOpen;
  late _gtt_engine_close_dart _engineClose;

  ffi.Pointer<ffi.Void>? _context;
  final StreamController<int> _priceUpdates = StreamController<int>.broadcast();
  Future<Map<String, dynamic>>? _priceUpdateInFlight;
  int _priceRevision = 0;
  bool _autoPriceRefreshStarted = false;
  Timer? _autoPriceRefreshTimer;

  static FfiBridge get instance => _instance ??= FfiBridge._();

  Stream<int> get priceUpdates => _priceUpdates.stream;
  int get priceRevision => _priceRevision;

  /// Starts a quiet online price refresh and keeps it on a 12-hour cadence.
  /// Call after the initial scan so the price writer does not contend with it.
  void startAutomaticPriceRefresh() {
    if (_autoPriceRefreshStarted) return;
    _autoPriceRefreshStarted = true;
    unawaited(_refreshPricesAutomatically());
  }

  Future<void> _refreshPricesAutomatically() async {
    try {
      await updatePrices();
    } catch (_) {
      // Keep automatic sync quiet. The pricing page still exposes the manual
      // action and the timer retries after the same interval.
      _scheduleAutomaticPriceRefresh();
    }
  }

  void _scheduleAutomaticPriceRefresh() {
    if (!_autoPriceRefreshStarted) return;
    _autoPriceRefreshTimer?.cancel();
    _autoPriceRefreshTimer = Timer(_autoPriceRefreshInterval, () {
      unawaited(_refreshPricesAutomatically());
    });
  }

  FfiBridge._() {
    _loadDylib();
    _bindFunctions();
  }

  void _loadDylib() {
    // The release package ships globaltokentracker_ffi.dll next to
    // globaltokentracker_ui.exe, so the executable's own directory is the
    // primary lookup location. `flutter run` keeps it in the working
    // directory or in a cargo target/ dir instead, hence the extra
    // candidates. All paths are relative to the runtime layout so the
    // app works on any machine — never hardcode a developer's absolute
    // path here.
    const dllName = 'globaltokentracker_ffi.dll';
    final exeDir = File(Platform.resolvedExecutable).parent;
    final possiblePaths = [
      exeDir.childFile(dllName).path,
      dllName,
      'target\\release\\$dllName',
      'target\\debug\\$dllName',
    ];

    for (final p in possiblePaths) {
      if (File(p).existsSync()) {
        _resolvedDylibPath = File(p).absolute.path;
        _dylib = ffi.DynamicLibrary.open(_resolvedDylibPath);
        return;
      }
    }

    // Last resort: let the OS search its default locations (app dir, cwd,
    // PATH). If that fails, surface a diagnosable error rather than an
    // opaque dynamic-library failure.
    _resolvedDylibPath = dllName;
    try {
      _dylib = ffi.DynamicLibrary.open(_resolvedDylibPath);
    } catch (e) {
      throw Exception(
        '$dllName not found. Expected it next to the executable or in the '
        'working directory. ($e)',
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
    _autoPriceRefreshTimer?.cancel();
    _autoPriceRefreshTimer = null;
    _autoPriceRefreshStarted = false;
    if (_context != null && _context!.address != 0) {
      _engineClose(_context!);
      _context = null;
    }
  }

  /// Offload the full incremental scan to a background worker isolate.
  ///
  /// The scan walks every registered adapter; a first pass over a newly added
  /// source can take tens of seconds (186 Antigravity databases on a real
  /// machine), and this call used to run synchronously on the UI isolate,
  /// freezing the whole app until it finished.
  Future<Map<String, dynamic>> scan() async {
    if (_context == null) throw Exception("Engine not initialized");
    final ctxAddress = _context!.address;
    final dylibPath = _resolvedDylibPath;

    return await Isolate.run(() {
      final dylib = ffi.DynamicLibrary.open(dylibPath);
      final scanFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_engine_scan_c>>('gtt_engine_scan')
          .asFunction<_gtt_engine_scan_dart>();
      final freeStringFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_free_string_c>>('gtt_free_string')
          .asFunction<_gtt_free_string_dart>();

      final ctx = ffi.Pointer<ffi.Void>.fromAddress(ctxAddress);
      final ptr = scanFunc(ctx);
      if (ptr == ffi.nullptr) throw Exception("Scan failed");
      final raw = ptr.toDartString();
      freeStringFunc(ptr);

      return jsonDecode(raw) as Map<String, dynamic>;
    });
  }

  /// Offload Overview retrieval and JSON deserialization to a background worker isolate.
  /// Main UI thread stays 100% fluid at 120fps!
  Future<OverviewData> getOverview({
    String rangeKey = 'week',
    int customStartMs = 0,
    int customEndMs = 0,
    List<String>? filterApps,
    List<String>? filterModels,
  }) async {
    if (_context == null) throw Exception("Engine not initialized");
    final ctxAddress = _context!.address;
    final dylibPath = _resolvedDylibPath;

    return await Isolate.run(() {
      final dylib = ffi.DynamicLibrary.open(dylibPath);
      final getOverviewFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_get_overview_c>>('gtt_get_overview')
          .asFunction<_gtt_get_overview_dart>();
      final freeStringFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_free_string_c>>('gtt_free_string')
          .asFunction<_gtt_free_string_dart>();

      final ctx = ffi.Pointer<ffi.Void>.fromAddress(ctxAddress);
      final rangePtr = rangeKey.toNativeUtf8();
      final appsPtr = filterApps != null && filterApps.isNotEmpty
          ? jsonEncode(filterApps).toNativeUtf8()
          : ffi.nullptr;
      final modelsPtr = filterModels != null && filterModels.isNotEmpty
          ? jsonEncode(filterModels).toNativeUtf8()
          : ffi.nullptr;

      final ptr = getOverviewFunc(
        ctx,
        rangePtr,
        customStartMs,
        customEndMs,
        appsPtr,
        modelsPtr,
      );

      calloc.free(rangePtr);
      if (appsPtr != ffi.nullptr) calloc.free(appsPtr);
      if (modelsPtr != ffi.nullptr) calloc.free(modelsPtr);

      if (ptr == ffi.nullptr) throw Exception("Failed to get overview data");
      final raw = ptr.toDartString();
      freeStringFunc(ptr);

      final json = jsonDecode(raw) as Map<String, dynamic>;
      return OverviewData.fromJson(json);
    });
  }

  /// Offload Details retrieval and JSON deserialization to a background worker isolate.
  Future<DetailData> getDetails({
    int page = 0,
    int pageSize = 50,
    List<String>? filterApps,
    List<String>? filterModels,
  }) async {
    if (_context == null) throw Exception("Engine not initialized");
    final ctxAddress = _context!.address;
    final dylibPath = _resolvedDylibPath;

    return await Isolate.run(() {
      final dylib = ffi.DynamicLibrary.open(dylibPath);
      final getDetailsFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_get_details_c>>('gtt_get_details')
          .asFunction<_gtt_get_details_dart>();
      final freeStringFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_free_string_c>>('gtt_free_string')
          .asFunction<_gtt_free_string_dart>();

      final ctx = ffi.Pointer<ffi.Void>.fromAddress(ctxAddress);
      final appsPtr = filterApps != null && filterApps.isNotEmpty
          ? jsonEncode(filterApps).toNativeUtf8()
          : ffi.nullptr;
      final modelsPtr = filterModels != null && filterModels.isNotEmpty
          ? jsonEncode(filterModels).toNativeUtf8()
          : ffi.nullptr;

      final ptr = getDetailsFunc(ctx, page, pageSize, appsPtr, modelsPtr);

      if (appsPtr != ffi.nullptr) calloc.free(appsPtr);
      if (modelsPtr != ffi.nullptr) calloc.free(modelsPtr);

      if (ptr == ffi.nullptr) return DetailData(rows: [], totalEvents: 0);
      final raw = ptr.toDartString();
      freeStringFunc(ptr);

      final json = jsonDecode(raw) as Map<String, dynamic>;
      return DetailData.fromJson(json);
    });
  }

  /// Offload Sources retrieval to background worker isolate.
  Future<List<SourceHealth>> getSources() async {
    if (_context == null) throw Exception("Engine not initialized");
    final ctxAddress = _context!.address;
    final dylibPath = _resolvedDylibPath;

    return await Isolate.run(() {
      final dylib = ffi.DynamicLibrary.open(dylibPath);
      final getSourcesFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_get_sources_c>>('gtt_get_sources')
          .asFunction<_gtt_get_sources_dart>();
      final freeStringFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_free_string_c>>('gtt_free_string')
          .asFunction<_gtt_free_string_dart>();

      final ctx = ffi.Pointer<ffi.Void>.fromAddress(ctxAddress);
      final ptr = getSourcesFunc(ctx);
      if (ptr == ffi.nullptr) return <SourceHealth>[];
      final raw = ptr.toDartString();
      freeStringFunc(ptr);

      final list = jsonDecode(raw) as List;
      return list.map((e) => SourceHealth.fromJson(e as Map<String, dynamic>)).toList();
    });
  }

  /// Offload Price Catalog retrieval (thousands of rows) to background worker isolate.
  Future<List<PriceRow>> getPrices({int limit = 5000}) async {
    if (_context == null) throw Exception("Engine not initialized");
    final ctxAddress = _context!.address;
    final dylibPath = _resolvedDylibPath;

    return await Isolate.run(() {
      final dylib = ffi.DynamicLibrary.open(dylibPath);
      final getPricesFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_get_prices_c>>('gtt_get_prices')
          .asFunction<_gtt_get_prices_dart>();
      final freeStringFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_free_string_c>>('gtt_free_string')
          .asFunction<_gtt_free_string_dart>();

      final ctx = ffi.Pointer<ffi.Void>.fromAddress(ctxAddress);
      final ptr = getPricesFunc(ctx, limit);
      if (ptr == ffi.nullptr) return <PriceRow>[];

      final raw = ptr.toDartString();
      freeStringFunc(ptr);

      final list = jsonDecode(raw) as List;
      return list.map((e) => PriceRow.fromJson(e as Map<String, dynamic>)).toList();
    });
  }

  /// Offload the price-feed refresh (network downloads plus parsing of
  /// multi-megabyte JSON) to a background worker isolate.
  Future<Map<String, dynamic>> updatePrices() async {
    final running = _priceUpdateInFlight;
    if (running != null) return running;

    late final Future<Map<String, dynamic>> update;
    update = _runPriceUpdate().then((result) {
      _priceRevision++;
      _priceUpdates.add(_priceRevision);
      _scheduleAutomaticPriceRefresh();
      return result;
    }).whenComplete(() {
      if (identical(_priceUpdateInFlight, update)) {
        _priceUpdateInFlight = null;
      }
    });
    _priceUpdateInFlight = update;
    return update;
  }

  Future<Map<String, dynamic>> _runPriceUpdate() async {
    if (_context == null) throw Exception("Engine not initialized");
    final ctxAddress = _context!.address;
    final dylibPath = _resolvedDylibPath;

    return await Isolate.run(() {
      final dylib = ffi.DynamicLibrary.open(dylibPath);
      final updatePricesFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_update_prices_c>>('gtt_update_prices')
          .asFunction<_gtt_update_prices_dart>();
      final freeStringFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_free_string_c>>('gtt_free_string')
          .asFunction<_gtt_free_string_dart>();

      final ctx = ffi.Pointer<ffi.Void>.fromAddress(ctxAddress);
      final ptr = updatePricesFunc(ctx);
      if (ptr == ffi.nullptr) throw Exception("Price update failed");
      final raw = ptr.toDartString();
      freeStringFunc(ptr);

      final result = jsonDecode(raw) as Map<String, dynamic>;
      if (result['ok'] == false) {
        throw Exception(result['error'] ?? 'Price update failed');
      }
      return result;
    });
  }

  /// Offload the vendor quota polling (network) to a background worker isolate.
  Future<Map<String, dynamic>> pollQuotas() async {
    if (_context == null) throw Exception("Engine not initialized");
    final ctxAddress = _context!.address;
    final dylibPath = _resolvedDylibPath;

    return await Isolate.run(() {
      final dylib = ffi.DynamicLibrary.open(dylibPath);
      final pollQuotasFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_poll_quotas_c>>('gtt_poll_quotas')
          .asFunction<_gtt_poll_quotas_dart>();
      final freeStringFunc = dylib
          .lookup<ffi.NativeFunction<_gtt_free_string_c>>('gtt_free_string')
          .asFunction<_gtt_free_string_dart>();

      final ctx = ffi.Pointer<ffi.Void>.fromAddress(ctxAddress);
      final ptr = pollQuotasFunc(ctx);
      if (ptr == ffi.nullptr) throw Exception("Quota poll failed");
      final raw = ptr.toDartString();
      freeStringFunc(ptr);

      return jsonDecode(raw) as Map<String, dynamic>;
    });
  }
}
