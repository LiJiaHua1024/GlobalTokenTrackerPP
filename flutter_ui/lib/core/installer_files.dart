import 'dart:io';

import 'package:crypto/crypto.dart';

class InstallFilesException implements Exception {
  InstallFilesException(this.message);
  final String message;

  @override
  String toString() => message;
}

/// Stage the complete bundle before changing any installed file. Backups stay
/// on the same volume so replacement and rollback can use rename, not copy.
Future<void> installBundle({
  required Directory source,
  required Directory destination,
  File? installerExecutable,
  void Function(String name, int completed, int total)? onProgress,
  int renameAttempts = 40,
  Duration retryDelay = const Duration(milliseconds: 250),
}) async {
  if (renameAttempts < 1) throw ArgumentError.value(renameAttempts);
  await destination.create(recursive: true);
  final sourcePath = await source.resolveSymbolicLinks();
  final destinationPath = await destination.resolveSymbolicLinks();
  final src = Directory(sourcePath);
  final dest = Directory(destinationPath);
  String directoryKey(String path) {
    final key = Directory(path).absolute.uri.normalizePath().toString();
    return Platform.isWindows ? key.toLowerCase() : key;
  }

  final sourceKey = directoryKey(sourcePath);
  final destinationKey = directoryKey(destinationPath);
  if (sourceKey.startsWith(destinationKey) ||
      destinationKey.startsWith(sourceKey)) {
    throw InstallFilesException('安装源与目标目录不能相同或互相包含，请重新运行完整安装包。');
  }

  final files = <String, File>{};
  await for (final entity in src.list(recursive: true, followLinks: false)) {
    if (entity is Link) {
      throw InstallFilesException('安装包包含不支持的链接：${entity.path}');
    }
    if (entity is! File) continue;
    final name = entity.path.substring(sourcePath.length + 1);
    if (name.toLowerCase() == 'installer_ui.exe') continue;
    files[name] = entity;
  }
  if (files.isEmpty) throw InstallFilesException('安装包中没有程序文件。');
  if (installerExecutable != null) {
    final installedSetup = File('${dest.path}/globaltokentrackerpp-setup.exe');
    if (!await installedSetup.exists() ||
        !await FileSystemEntity.identical(
            installedSetup.path, installerExecutable.path)) {
      files['globaltokentrackerpp-setup.exe'] = installerExecutable;
    }
  }
  final orderedFiles = files.entries.toList()
    ..sort((a, b) => a.key.compareTo(b.key));

  final transaction = await dest.createTemp('.gtt-install-');
  final entries = <_InstallFile>[];
  var preserveBackup = false;
  String? currentFile;

  Future<void> move(File file, String target) async {
    for (var attempt = 1;; attempt++) {
      try {
        await file.rename(target);
        return;
      } on FileSystemException {
        if (attempt >= renameAttempts) rethrow;
        await Future<void>.delayed(retryDelay);
      }
    }
  }

  try {
    // A directory/file collision is detected before any old file is replaced.
    for (final item in orderedFiles) {
      currentFile = item.key;
      final target = File('${dest.path}/${item.key}');
      final type = await FileSystemEntity.type(target.path, followLinks: false);
      if (type != FileSystemEntityType.notFound &&
          type != FileSystemEntityType.file) {
        throw InstallFilesException('目标路径已存在且不是普通文件：${target.path}');
      }
      var parent = target.parent;
      while (directoryKey(parent.path) != destinationKey) {
        final parentType =
            await FileSystemEntity.type(parent.path, followLinks: false);
        if (parentType != FileSystemEntityType.notFound &&
            parentType != FileSystemEntityType.directory) {
          throw InstallFilesException('目标父目录已被文件或链接占用：${parent.path}');
        }
        parent = parent.parent;
      }
      final staged = File('${transaction.path}/new/${item.key}');
      await staged.parent.create(recursive: true);
      final expected = await sha256.bind(item.value.openRead()).first;
      await item.value.copy(staged.path);
      if (await sha256.bind(staged.openRead()).first != expected) {
        throw InstallFilesException('安装文件校验失败：${item.key}');
      }
      entries.add(_InstallFile(
        target,
        staged,
        File('${transaction.path}/old/${item.key}'),
      ));
      onProgress?.call(item.key, entries.length, files.length * 2);
    }

    for (var i = 0; i < entries.length; i++) {
      final entry = entries[i];
      currentFile = entry.target.path;
      await entry.target.parent.create(recursive: true);
      if (await entry.target.exists()) {
        await entry.backup.parent.create(recursive: true);
        await move(entry.target, entry.backup.path);
        entry.backedUp = true;
      }
      await move(entry.staged, entry.target.path);
      entry.installed = true;
      onProgress?.call(
          orderedFiles[i].key, files.length + i + 1, files.length * 2);
    }
  } catch (error) {
    final rollbackErrors = <String>[];
    for (final entry in entries.reversed) {
      try {
        if (entry.installed) {
          // Move the new file aside before restoring the old one. Never delete
          // a backup to make rollback succeed.
          await move(entry.target, entry.staged.path);
        }
        if (entry.backedUp) await move(entry.backup, entry.target.path);
      } catch (rollbackError) {
        rollbackErrors.add('${entry.target.path}: $rollbackError');
      }
    }
    preserveBackup = rollbackErrors.isNotEmpty;
    throw InstallFilesException(
      '无法更新文件：$currentFile\n'
      '请关闭 GlobalTokenTracker++ 及其他占用该目录的程序后重试。\n'
      '${preserveBackup ? '部分文件无法恢复，旧文件备份保留在：${transaction.path}\n${rollbackErrors.join('\n')}' : '本次文件更改已撤回，原有文件已保留。'}\n'
      '详细原因：$error',
    );
  } finally {
    if (!preserveBackup) {
      try {
        await transaction.delete(recursive: true);
      } on FileSystemException {
        // Cleanup failure must not turn a completed upgrade into an error.
      }
    }
  }
}

class _InstallFile {
  _InstallFile(this.target, this.staged, this.backup);
  final File target;
  final File staged;
  final File backup;
  bool backedUp = false;
  bool installed = false;
}
