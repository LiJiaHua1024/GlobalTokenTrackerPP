import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'dart:async';
import 'ffi_bridge.dart';

class ThemeProvider extends ChangeNotifier {
  Map<String, dynamic> _fx = const {'display': 'USD', 'usd_to': 1.0};
  bool currencyBusy = false;
  String? currencyError;
  Timer? _currencyTimer;
  bool _disposed = false;
  String get requestedCurrency => _fx['display'] as String? ?? 'USD';
  bool get hasExchangeRate =>
      requestedCurrency == 'USD' ||
      (_fx['source'] != null && (_fx['usd_to'] as num? ?? 0) > 0);
  String get displayCurrency => hasExchangeRate ? requestedCurrency : 'USD';
  String? get exchangeSource => _fx['source'] as String?;
  int? get exchangeUpdatedAt => _fx['fetched_at_ms'] as int?;
  bool get exchangeStale => _fx['stale'] == true;
  void applyCurrencyStatus(Map<String, dynamic> state) {
    if (_disposed) return;
    _fx = state;
    notifyListeners();
  }

  String formatMoney(num usd, {int decimals = 2}) {
    final code = displayCurrency;
    const symbols = {
      'USD': r'$',
      'CNY': '¥',
      'EUR': '€',
      'GBP': '£',
      'JPY': '¥',
      'HKD': r'HK$',
      'SGD': r'S$',
      'AUD': r'A$',
      'CAD': r'C$',
      'KRW': '₩'
    };
    final rate =
        hasExchangeRate ? (_fx['usd_to'] as num? ?? 1).toDouble() : 1.0;
    return '${symbols[code] ?? '$code '}${(usd * rate).toStringAsFixed(decimals)}';
  }

  Future<void> initializeCurrency() async {
    try {
      applyCurrencyStatus(
          await FfiBridge.instance.queryJson('gtt_fx', {'action': 'status'}));
      if (exchangeSource == null || exchangeStale) await refreshCurrency();
    } catch (e) {
      if (!_disposed) {
        currencyError = e.toString();
        notifyListeners();
      }
    }
    if (!_disposed) {
      _currencyTimer?.cancel();
      _currencyTimer =
          Timer.periodic(const Duration(hours: 12), (_) => refreshCurrency());
    }
  }

  Future<void> setCurrency(String code) async {
    applyCurrencyStatus(await FfiBridge.instance
        .queryJson('gtt_fx', {'action': 'set', 'currency': code}));
    if (!hasExchangeRate || exchangeStale) await refreshCurrency();
  }

  Future<void> refreshCurrency() async {
    if (currencyBusy || _disposed) return;
    currencyBusy = true;
    currencyError = null;
    notifyListeners();
    try {
      applyCurrencyStatus(
          await FfiBridge.instance.queryJson('gtt_fx', {'action': 'refresh'}));
    } catch (e) {
      if (!_disposed) currencyError = e.toString();
    } finally {
      if (!_disposed) {
        currencyBusy = false;
        notifyListeners();
      }
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _currencyTimer?.cancel();
    super.dispose();
  }

  ThemeMode _themeMode = ThemeMode.system;
  Color _seedColor = const Color(0xFF1A73E8); // Google Blue M3
  bool _compactNumbers = true;

  double _uiScale = 1.0;

  ThemeMode get themeMode => _themeMode;
  Color get seedColor => _seedColor;
  bool get compactNumbers => _compactNumbers;
  double get uiScale => _uiScale;

  void setThemeMode(ThemeMode mode) {
    _themeMode = mode;
    notifyListeners();
  }

  void setSeedColor(Color color) {
    _seedColor = color;
    notifyListeners();
  }

  void setCompactNumbers(bool value) {
    _compactNumbers = value;
    notifyListeners();
  }

  void setUiScale(double scale) {
    _uiScale = scale.clamp(0.75, 2.0);
    notifyListeners();
  }

  /// Formats numbers to K, M, B, T when compactNumbers is enabled
  String formatTokens(num value) {
    if (!_compactNumbers) {
      return NumberFormat('#,###').format(value);
    }
    final double v = value.toDouble();
    final absV = v.abs();
    if (absV >= 1000000000000) {
      return '${(v / 1000000000000).toStringAsFixed(2)}T';
    } else if (absV >= 1000000000) {
      return '${(v / 1000000000).toStringAsFixed(2)}B';
    } else if (absV >= 1000000) {
      return '${(v / 1000000).toStringAsFixed(2)}M';
    } else if (absV >= 1000) {
      return '${(v / 1000).toStringAsFixed(1)}K';
    } else {
      return NumberFormat('#,##0').format(value);
    }
  }

  ThemeData get lightTheme {
    return ThemeData(
      useMaterial3: true,
      colorScheme: ColorScheme.fromSeed(
        seedColor: _seedColor,
        brightness: Brightness.light,
      ),
      fontFamily: 'Segoe UI',
      cardTheme: CardThemeData(
        elevation: 1,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
        clipBehavior: Clip.antiAlias,
      ),
      navigationRailTheme: const NavigationRailThemeData(
        labelType: NavigationRailLabelType.all,
        groupAlignment: -0.9,
      ),
    );
  }

  ThemeData get darkTheme {
    return ThemeData(
      useMaterial3: true,
      colorScheme: ColorScheme.fromSeed(
        seedColor: _seedColor,
        brightness: Brightness.dark,
      ),
      fontFamily: 'Segoe UI',
      cardTheme: CardThemeData(
        elevation: 1,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
        clipBehavior: Clip.antiAlias,
      ),
      navigationRailTheme: const NavigationRailThemeData(
        labelType: NavigationRailLabelType.all,
        groupAlignment: -0.9,
      ),
    );
  }
}
