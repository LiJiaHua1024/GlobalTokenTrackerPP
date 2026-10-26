import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/native_runtime.dart';
import 'package:globaltokentracker_ui/widgets/startup_error_app.dart';

void main() {
  test('a loadable library without the required ABI cannot be used as fallback',
      () {
    expect(
      () => openVerifiedNativeLibrary('kernel32.dll'),
      throwsA(isA<NativeRuntimeException>().having(
          (error) => error.toString(),
          'repair instructions',
          allOf(
              contains('重新安装'), contains('gtt_fx'), contains('kernel32.dll')))),
    );
  }, skip: !Platform.isWindows);

  testWidgets('startup failure shows a repair message without starting the UI',
      (tester) async {
    final message =
        NativeRuntimeException('globaltokentracker_ffi.dll\n缺少接口：gtt_fx');
    await tester.pumpWidget(StartupErrorApp(message: message.toString()));
    await tester.pumpAndSettle();
    expect(find.text('无法启动 GlobalTokenTracker++'), findsOneWidget);
    expect(find.text(message.toString()), findsOneWidget);
    expect(find.text('总览'), findsNothing);
    expect(tester.takeException(), isNull);
  });
}
