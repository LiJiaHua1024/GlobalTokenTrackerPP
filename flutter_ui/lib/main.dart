import 'package:animations/animations.dart';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:window_manager/window_manager.dart';

import 'core/ffi_bridge.dart';
import 'core/theme.dart';
import 'pages/details_page.dart';
import 'pages/overview_page.dart';
import 'pages/installer_page.dart';
import 'pages/pricing_page.dart';
import 'pages/quotas_page.dart';
import 'pages/settings_page.dart';
import 'pages/sources_page.dart';
import 'widgets/custom_title_bar.dart';

void main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();

  final isInstaller = args.contains('--setup') || args.contains('--uninstall');

  // Initialize Desktop Window Manager for seamless custom titlebar
  try {
    await windowManager.ensureInitialized();

    final windowOptions = WindowOptions(
      size: isInstaller ? const Size(660, 520) : const Size(1300, 820),
      minimumSize: isInstaller ? const Size(660, 520) : const Size(960, 640),
      maximumSize: isInstaller ? const Size(660, 520) : null,
      center: true,
      backgroundColor: Colors.transparent,
      skipTaskbar: false,
      titleBarStyle: TitleBarStyle.hidden,
    );

    windowManager.waitUntilReadyToShow(windowOptions, () async {
      await windowManager.setResizable(!isInstaller);
      await windowManager.show();
      await windowManager.focus();
    });
  } catch (e) {
    debugPrint("Failed to initialize windowManager: $e");
  }

  if (isInstaller) {
    runApp(InstallerApp(args: args));
    return;
  }

  // Initialize native Rust engine via FFI
  try {
    FfiBridge.instance.openEngine();
  } catch (e) {
    debugPrint("Failed to initialize FFI engine on startup: $e");
  }

  runApp(
    ChangeNotifierProvider(
      create: (_) => ThemeProvider(),
      child: const GlobalTokenTrackerApp(),
    ),
  );
}

class GlobalTokenTrackerApp extends StatelessWidget {
  const GlobalTokenTrackerApp({super.key});

  @override
  Widget build(BuildContext context) {
    final themeProvider = Provider.of<ThemeProvider>(context);

    return MaterialApp(
      title: 'GlobalTokenTracker++',
      debugShowCheckedModeBanner: false,
      themeMode: themeProvider.themeMode,
      theme: themeProvider.lightTheme,
      darkTheme: themeProvider.darkTheme,
      home: const MainShell(),
    );
  }
}

class MainShell extends StatefulWidget {
  const MainShell({super.key});

  @override
  State<MainShell> createState() => _MainShellState();
}

class _MainShellState extends State<MainShell> {
  int _selectedIndex = 0;

  final List<Widget> _pages = const [
    OverviewPage(),
    DetailsPage(),
    QuotasPage(),
    SourcesPage(),
    PricingPage(),
    SettingsPage(),
  ];

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Column(
        children: [
          // Integrated Material 3 Title Bar (Draggable, Seamless, Maximize/Close buttons)
          const CustomTitleBar(),

          // Main Window Layout
          Expanded(
            child: Row(
              children: [
                // Desktop Navigation Rail
                NavigationRail(
                  selectedIndex: _selectedIndex,
                  onDestinationSelected: (int index) {
                    setState(() {
                      _selectedIndex = index;
                    });
                  },
                  destinations: const [
                    NavigationRailDestination(
                      icon: Icon(Icons.dashboard_outlined),
                      selectedIcon: Icon(Icons.dashboard),
                      label: Text('总览'),
                    ),
                    NavigationRailDestination(
                      icon: Icon(Icons.table_rows_outlined),
                      selectedIcon: Icon(Icons.table_rows),
                      label: Text('明细'),
                    ),
                    NavigationRailDestination(
                      icon: Icon(Icons.pie_chart_outline),
                      selectedIcon: Icon(Icons.pie_chart),
                      label: Text('配额'),
                    ),
                    NavigationRailDestination(
                      icon: Icon(Icons.hub_outlined),
                      selectedIcon: Icon(Icons.hub),
                      label: Text('数据源'),
                    ),
                    NavigationRailDestination(
                      icon: Icon(Icons.monetization_on_outlined),
                      selectedIcon: Icon(Icons.monetization_on),
                      label: Text('价格表'),
                    ),
                    NavigationRailDestination(
                      icon: Icon(Icons.settings_outlined),
                      selectedIcon: Icon(Icons.settings),
                      label: Text('设置'),
                    ),
                  ],
                ),
                const VerticalDivider(thickness: 1, width: 1),

                // Main Content View with Material Motion Page Transition
                Expanded(
                  child: PageTransitionSwitcher(
                    duration: const Duration(milliseconds: 300),
                    transitionBuilder:
                        (child, primaryAnimation, secondaryAnimation) {
                      return SharedAxisTransition(
                        animation: primaryAnimation,
                        secondaryAnimation: secondaryAnimation,
                        transitionType: SharedAxisTransitionType.vertical,
                        child: child,
                      );
                    },
                    child: _pages[_selectedIndex],
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
