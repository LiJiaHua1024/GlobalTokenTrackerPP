import 'dart:io';

import '../flutter_ui/lib/core/native_runtime.dart';

void main(List<String> args) {
  if (args.length != 1 || !File(args.single).existsSync()) {
    stderr.writeln('Expected the path to globaltokentracker_ffi.dll');
    exitCode = 1;
    return;
  }
  try {
    openVerifiedNativeLibrary(File(args.single).absolute.path);
    stdout.writeln(
      'Native runtime verified: ${requiredNativeSymbols.length} exports',
    );
  } catch (error) {
    stderr.writeln(error);
    exitCode = 1;
  }
}
