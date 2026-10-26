import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/installer_files.dart';

void main() {
  late Directory scratch;
  late Directory source;
  late Directory dest;

  setUp(() async {
    scratch = await Directory.systemTemp.createTemp('gtt-install-test-');
    source = await Directory('${scratch.path}/source').create();
    dest = await Directory('${scratch.path}/installed').create();
  });
  tearDown(() async => scratch.delete(recursive: true));

  Future<File> write(Directory dir, String name, String content) async {
    final file = File('${dir.path}/$name');
    await file.parent.create(recursive: true);
    return file.writeAsString(content);
  }

  Future<void> install({int attempts = 2}) => installBundle(
        source: source,
        destination: dest,
        renameAttempts: attempts,
        retryDelay: const Duration(milliseconds: 20),
      );

  test('upgrades existing files, nested data and the uninstaller together',
      () async {
    await write(source, 'app.exe', 'new ui');
    await write(source, 'core.dll', 'new core');
    await write(source, 'data/app.so', 'new assets');
    await write(source, 'installer_ui.exe', 'helper');
    final setup = await write(scratch, 'setup.exe', 'new setup');
    await write(dest, 'app.exe', 'old ui');
    await write(dest, 'core.dll', 'old core');
    await write(dest, 'globaltokentrackerpp-setup.exe', 'old setup');
    await write(dest, 'user-file.txt', 'keep');
    await installBundle(
        source: source, destination: dest, installerExecutable: setup);
    expect(await File('${dest.path}/app.exe').readAsString(), 'new ui');
    expect(await File('${dest.path}/core.dll').readAsString(), 'new core');
    expect(await File('${dest.path}/data/app.so').readAsString(), 'new assets');
    expect(
        await File('${dest.path}/globaltokentrackerpp-setup.exe')
            .readAsString(),
        'new setup');
    expect(await File('${dest.path}/user-file.txt').readAsString(), 'keep');
    expect(await File('${dest.path}/installer_ui.exe').exists(), isFalse);
    expect(await dest.list().any((e) => e.path.contains('.gtt-install-')),
        isFalse);
  });

  test('a directory collision leaves all existing files unchanged', () async {
    await write(source, 'app.exe', 'new');
    await write(source, 'core.dll', 'new');
    await write(dest, 'app.exe', 'old');
    await Directory('${dest.path}/core.dll').create();
    await expectLater(install(), throwsA(isA<InstallFilesException>()));
    expect(await File('${dest.path}/app.exe').readAsString(), 'old');
    expect(await Directory('${dest.path}/core.dll').exists(), isTrue);
  });

  test('same directory or nested destinations cannot overwrite their source',
      () async {
    await write(source, 'app.exe', 'source');
    await expectLater(installBundle(source: source, destination: source),
        throwsA(isA<InstallFilesException>()));
    await expectLater(
        installBundle(
            source: source, destination: Directory('${source.path}/nested')),
        throwsA(isA<InstallFilesException>()));
    expect(await File('${source.path}/app.exe').readAsString(), 'source');
  });

  test('the running installer in the target is preserved during repair',
      () async {
    await write(source, 'app.exe', 'new');
    final setup =
        await write(dest, 'globaltokentrackerpp-setup.exe', 'running setup');
    await installBundle(
        source: source, destination: dest, installerExecutable: setup);
    expect(await setup.readAsString(), 'running setup');
  });

  test('a locked DLL rolls back files already replaced and removes new files',
      () async {
    await write(source, 'app.exe', 'new ui');
    await write(source, 'added.txt', 'new file');
    await write(source, 'core.dll', 'new core');
    await write(dest, 'app.exe', 'old ui');
    final core = await write(dest, 'core.dll', 'old core');
    // Hold the DLL only after the other files have been installed, independent
    // of filesystem enumeration order. This reproduces a Windows sharing lock.
    void Function()? unlock;
    var replacements = 0;
    try {
      await expectLater(
        installBundle(
          source: source,
          destination: dest,
          renameAttempts: 2,
          retryDelay: const Duration(milliseconds: 10),
          onProgress: (name, done, total) {
            if (done <= total ~/ 2) return;
            replacements++;
            if (name != 'core.dll' && unlock == null) unlock = _lockFile(core);
          },
        ),
        throwsA(isA<InstallFilesException>()),
      );
      expect(replacements, greaterThan(0));
      expect(await File('${dest.path}/app.exe').readAsString(), 'old ui');
      expect(await File('${dest.path}/added.txt').exists(), isFalse);
      expect(await core.readAsString(), 'old core');
    } finally {
      unlock?.call();
    }
  }, skip: !Platform.isWindows);

  test('replacement retries until a Windows sharing lock is released',
      () async {
    await write(source, 'core.dll', 'new core');
    final core = await write(dest, 'core.dll', 'old core');
    final unlock = _lockFile(core);
    final release =
        Future<void>.delayed(const Duration(milliseconds: 80), unlock);
    try {
      await install(attempts: 30);
      expect(await core.readAsString(), 'new core');
    } finally {
      await release;
    }
  }, skip: !Platform.isWindows);
}

// Allow reading, but deny rename/delete while the handle is open, just like a
// running executable or a loaded native DLL on Windows.
void Function() _lockFile(File file) {
  final kernel = ffi.DynamicLibrary.open('kernel32.dll');
  final create = kernel.lookupFunction<
      ffi.IntPtr Function(ffi.Pointer<Utf16>, ffi.Uint32, ffi.Uint32,
          ffi.Pointer<ffi.Void>, ffi.Uint32, ffi.Uint32, ffi.IntPtr),
      int Function(ffi.Pointer<Utf16>, int, int, ffi.Pointer<ffi.Void>, int,
          int, int)>('CreateFileW');
  final close =
      kernel.lookupFunction<ffi.Int32 Function(ffi.IntPtr), int Function(int)>(
          'CloseHandle');
  final path = file.path.toNativeUtf16();
  try {
    final handle = create(path, 0x80000000, 1, ffi.nullptr, 3, 0x80, 0);
    if (handle == -1) throw StateError('Could not lock ${file.path}');
    return () => close(handle);
  } finally {
    calloc.free(path);
  }
}
