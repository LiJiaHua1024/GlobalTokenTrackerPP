import 'package:flutter/material.dart';

import '../core/theme.dart';
import 'custom_title_bar.dart';

class StartupErrorApp extends StatelessWidget {
  const StartupErrorApp({super.key, required this.message});
  final String message;

  @override
  Widget build(BuildContext context) {
    final theme = ThemeProvider();
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: theme.lightTheme,
      darkTheme: theme.darkTheme,
      home: Scaffold(
        body: Column(children: [
          const CustomTitleBar(),
          Expanded(
            child: Center(
              child: SingleChildScrollView(
                padding: const EdgeInsets.all(32),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 720),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const Text('无法启动 GlobalTokenTracker++',
                          style: TextStyle(fontSize: 24)),
                      const SizedBox(height: 16),
                      SelectableText(message),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ]),
      ),
    );
  }
}
