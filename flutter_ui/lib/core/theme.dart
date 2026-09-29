import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

class ThemeProvider extends ChangeNotifier {
  ThemeMode _themeMode = ThemeMode.system;
  Color _seedColor = const Color(0xFF1A73E8); // Google Blue M3
  bool _compactNumbers = true;

  ThemeMode get themeMode => _themeMode;
  Color get seedColor => _seedColor;
  bool get compactNumbers => _compactNumbers;

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
