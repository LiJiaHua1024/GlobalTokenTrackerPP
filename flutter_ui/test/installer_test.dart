import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/pages/installer_page.dart';

void main() {
  testWidgets('InstallerApp adapts to light and dark platform brightness', (WidgetTester tester) async {
    // 1. Test Light mode
    tester.platformDispatcher.platformBrightnessTestValue = Brightness.light;
    addTearDown(() {
      tester.platformDispatcher.clearPlatformBrightnessTestValue();
    });

    await tester.pumpWidget(const InstallerApp(args: ['--setup']));
    await tester.pumpAndSettle();

    final lightContext = tester.element(find.byType(Scaffold));
    final lightTheme = Theme.of(lightContext);
    expect(lightTheme.brightness, Brightness.light);
    final lightScaffold = tester.widget<Scaffold>(find.byType(Scaffold));
    expect(lightScaffold.backgroundColor, lightTheme.colorScheme.surface);

    // 2. Test Dark mode
    tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
    await tester.pumpAndSettle();

    final darkContext = tester.element(find.byType(Scaffold));
    final darkTheme = Theme.of(darkContext);
    expect(darkTheme.brightness, Brightness.dark);
    final darkScaffold = tester.widget<Scaffold>(find.byType(Scaffold));
    expect(darkScaffold.backgroundColor, darkTheme.colorScheme.surface);
  });
}
