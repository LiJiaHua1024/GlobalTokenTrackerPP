import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'app_info.dart';
import 'ffi_bridge.dart';
import 'semver.dart';
import 'update_models.dart';

enum UpdateStatus {
  idle,
  checking,
  available,
  upToDate,
  error,
}

enum DownloadStatus {
  idle,
  downloading,
  completed,
  failed,
}

class UpdateProvider extends ChangeNotifier {
  UpdateStatus _status = UpdateStatus.idle;
  DownloadStatus _downloadStatus = DownloadStatus.idle;

  UpdateInfo? _updateInfo;
  String? _errorMessage;
  UpdateSettings _settings = const UpdateSettings();

  bool _isDismissedForSession = false;

  // Download state
  double _downloadProgress = 0.0;
  int _downloadedBytes = 0;
  int _totalBytes = 0;
  String? _downloadedFilePath;
  HttpClient? _downloadClient;
  // Set by cancelDownload; the download loop and its error handler check it
  // so a force-closed socket can't surface as a scary "download failed".
  bool _downloadCancelled = false;

  Timer? _periodicCheckTimer;

  // Getters
  UpdateStatus get status => _status;
  DownloadStatus get downloadStatus => _downloadStatus;
  UpdateInfo? get updateInfo => _updateInfo;
  String? get errorMessage => _errorMessage;
  UpdateSettings get settings => _settings;
  bool get hasUpdate => _status == UpdateStatus.available && _updateInfo != null;
  bool get isChecking => _status == UpdateStatus.checking;
  bool get isDismissedForSession => _isDismissedForSession;

  double get downloadProgress => _downloadProgress;
  int get downloadedBytes => _downloadedBytes;
  int get totalBytes => _totalBytes;
  String? get downloadedFilePath => _downloadedFilePath;

  UpdateProvider() {
    _loadSettings();
  }

  @override
  void dispose() {
    _periodicCheckTimer?.cancel();
    _downloadClient?.close(force: true);
    super.dispose();
  }

  /// Get the configuration directory where settings.json is stored
  String _getConfigDir() {
    try {
      final dbPath = FfiBridge.instance.getDefaultDbPath();
      if (dbPath.isNotEmpty) {
        return File(dbPath).parent.path;
      }
    } catch (_) {}

    final home = Platform.environment['USERPROFILE'] ??
        Platform.environment['HOME'] ??
        Directory.current.path;
    return '$home\\.globaltokentracker';
  }

  File _getSettingsFile() {
    final dir = Directory(_getConfigDir());
    if (!dir.existsSync()) {
      dir.createSync(recursive: true);
    }
    return File('${dir.path}\\app_settings.json');
  }

  void _loadSettings() {
    try {
      final file = _getSettingsFile();
      if (file.existsSync()) {
        final content = file.readAsStringSync();
        final map = jsonDecode(content) as Map<String, dynamic>;
        _settings = UpdateSettings.fromJson(map);
      }
    } catch (e) {
      debugPrint('[UpdateProvider] Error loading settings: $e');
    }
  }

  void _saveSettings() {
    try {
      final file = _getSettingsFile();
      file.writeAsStringSync(jsonEncode(_settings.toJson()), flush: true);
    } catch (e) {
      debugPrint('[UpdateProvider] Error saving settings: $e');
    }
  }

  /// Initialize periodic update checking and startup check
  void initAutoCheck() {
    // 1. Gentle startup auto-check after 3 seconds (non-blocking, smooth UX)
    Timer(const Duration(seconds: 3), () {
      checkForUpdates(isAutoCheck: true);
    });

    // 2. Periodic background check every hour (evaluates check interval)
    _periodicCheckTimer?.cancel();
    _periodicCheckTimer = Timer.periodic(const Duration(hours: 1), (_) {
      checkForUpdates(isAutoCheck: true);
    });
  }

  /// Checks for updates.
  /// [isAutoCheck]: If true, runs silently without modal popups or error alerts.
  /// [force]: If true, ignores interval throttle.
  Future<UpdateInfo?> checkForUpdates({
    bool isAutoCheck = false,
    bool force = false,
  }) async {
    if (isAutoCheck) {
      if (!_settings.autoCheckEnabled) return null;
      if (_settings.checkInterval == UpdateCheckInterval.manual) return null;

      // Throttle based on interval setting
      if (!force && _settings.lastCheckTime != null) {
        final now = DateTime.now();
        final diffHours = now.difference(_settings.lastCheckTime!).inHours;
        if (diffHours < _settings.checkInterval.hours) {
          // Check interval has not elapsed yet
          return null;
        }
      }
    }

    _status = UpdateStatus.checking;
    _errorMessage = null;
    notifyListeners();

    try {
      final client = HttpClient();
      client.connectionTimeout = const Duration(seconds: 10);

      final uri = Uri.parse(AppInfo.releasesApiUrl);
      final request = await client.getUrl(uri);
      request.headers.set('User-Agent', AppInfo.userAgent);
      request.headers.set('Accept', 'application/vnd.github.v3+json');

      final response = await request.close();

      if (response.statusCode == 403) {
        throw Exception('GitHub API 访问频次达到限制，请稍后再试');
      } else if (response.statusCode == 404) {
        throw Exception('未找到版本发布仓库');
      } else if (response.statusCode != 200) {
        throw Exception('检查更新失败 (HTTP ${response.statusCode})');
      }

      final body = await response.transform(utf8.decoder).join();
      final dynamic decoded = jsonDecode(body);

      if (decoded is! List) {
        throw Exception('GitHub 返回的数据格式异常');
      }

      UpdateInfo? candidate;
      for (final item in decoded) {
        if (item is Map<String, dynamic>) {
          final isPrerelease = item['prerelease'] as bool? ?? false;
          if (!_settings.includePrerelease && isPrerelease) {
            continue;
          }

          final update = UpdateInfo.fromGitHubJson(item);
          if (SemVer.isNewer(update.version, AppInfo.currentVersion)) {
            candidate = update;
            break;
          }
        }
      }

      // Update last check time
      _settings = _settings.copyWith(lastCheckTime: DateTime.now());
      _saveSettings();

      if (candidate != null) {
        _updateInfo = candidate;
        _status = UpdateStatus.available;

        // If this version was explicitly skipped by user and it's an auto-check, suppress banner
        if (isAutoCheck && _settings.skippedVersion == candidate.version) {
          _isDismissedForSession = true;
        }
      } else {
        _updateInfo = null;
        _status = UpdateStatus.upToDate;
      }

      notifyListeners();
      return candidate;
    } catch (e) {
      debugPrint('[UpdateProvider] Update check error: $e');
      if (isAutoCheck) {
        // Fail silently during background auto-check to never disturb user
        _status = UpdateStatus.idle;
      } else {
        _status = UpdateStatus.error;
        _errorMessage = e.toString().replaceFirst(RegExp(r'^Exception:\s*'), '');
      }
      notifyListeners();
      return null;
    }
  }

  /// Download the setup installer with live progress
  Future<void> startDownload() async {
    final asset = _updateInfo?.setupAsset ?? _updateInfo?.primaryAsset;
    if (asset == null || asset.downloadUrl.isEmpty) {
      _errorMessage = '当前发布未包含适用的 Windows 安装包';
      _downloadStatus = DownloadStatus.failed;
      notifyListeners();
      return;
    }

    _downloadStatus = DownloadStatus.downloading;
    _downloadProgress = 0.0;
    _downloadedBytes = 0;
    _totalBytes = asset.size;
    _errorMessage = null;
    _downloadCancelled = false;
    notifyListeners();

    // Hoisted so the finally below can remove a partial installer when the
    // download is cancelled or fails midway.
    File? saveFile;
    try {
      final tempDir = Directory.systemTemp;
      final rawName = asset.name.isNotEmpty
          ? asset.name
          : 'GlobalTokenTrackerPP-Setup-${_updateInfo!.version}-win-x64.exe';
      // asset.name comes from release JSON — basename it so a tampered name
      // can never escape %TEMP%.
      final fileName = rawName.split(RegExp(r'[\\/]')).last;
      saveFile = File('${tempDir.path}\\$fileName');

      _downloadClient?.close(force: true);
      _downloadClient = HttpClient();

      final request = await _downloadClient!.getUrl(Uri.parse(asset.downloadUrl));
      request.headers.set('User-Agent', AppInfo.userAgent);

      final response = await request.close();

      // Handle HTTP redirects (GitHub Releases redirect to AWS S3/objects)
      HttpClientResponse finalResponse = response;
      if (response.isRedirect) {
        final redirectUri = response.headers.value(HttpHeaders.locationHeader);
        if (redirectUri != null) {
          final redirectReq = await _downloadClient!.getUrl(Uri.parse(redirectUri));
          finalResponse = await redirectReq.close();
        }
      }

      if (finalResponse.statusCode != 200) {
        throw Exception('下载失败 (HTTP ${finalResponse.statusCode})');
      }

      final contentLength = finalResponse.contentLength;
      if (contentLength > 0) {
        _totalBytes = contentLength;
      }

      final sink = saveFile.openWrite();
      await for (final chunk in finalResponse) {
        if (_downloadCancelled) break;
        sink.add(chunk);
        _downloadedBytes += chunk.length;
        if (_totalBytes > 0) {
          _downloadProgress = (_downloadedBytes / _totalBytes).clamp(0.0, 1.0);
        }
        notifyListeners();
      }

      await sink.flush();
      await sink.close();

      // Cancelled: the partial installer must never linger in %TEMP%, and
      // there is nothing left to digest-verify.
      if (_downloadCancelled) {
        await saveFile.delete();
        _downloadStatus = DownloadStatus.idle;
        _downloadProgress = 0.0;
        _downloadedBytes = 0;
        notifyListeners();
        return;
      }

      // GitHub stamps release assets with `digest: "sha256:<hex>"`. Verify
      // before the file can ever be executed; on mismatch delete it and fail
      // loudly instead of handing over a bogus installer.
      final expected = _expectedSha256(asset.digest);
      if (expected != null) {
        final actual =
            sha256.convert(await saveFile.readAsBytes()).toString();
        if (actual != expected) {
          await saveFile.delete();
          throw Exception('安装包 SHA256 校验失败，已删除下载文件');
        }
      }

      _downloadedFilePath = saveFile.path;
      _downloadStatus = DownloadStatus.completed;
      notifyListeners();
    } catch (e) {
      debugPrint('[UpdateProvider] Download error: $e');
      if (_downloadCancelled) {
        // The force-closed socket surfaces here as an exception — the user
        // asked for this; show idle, not a failure.
        _downloadStatus = DownloadStatus.idle;
        _errorMessage = null;
      } else {
        _downloadStatus = DownloadStatus.failed;
        _errorMessage = '下载失败: ${e.toString().replaceFirst(RegExp(r'^Exception:\s*'), '')}';
      }
      notifyListeners();
    } finally {
      // A cancelled or failed download must not leave a truncated installer
      // in %TEMP%.
      final f = saveFile;
      if (f != null && _downloadStatus != DownloadStatus.completed) {
        try {
          if (await f.exists()) await f.delete();
        } catch (_) {}
      }
    }
  }

  /// Cancels any active download
  void cancelDownload() {
    _downloadCancelled = true;
    _downloadClient?.close(force: true);
    _downloadClient = null;
    _downloadStatus = DownloadStatus.idle;
    _downloadProgress = 0.0;
    _downloadedBytes = 0;
    notifyListeners();
  }

  /// `digest` is `"sha256:<hex>"` — lowercase hex, or null when absent
  /// (older releases carry no digest; the Authenticode check then gates).
  String? _expectedSha256(String? digest) {
    if (digest == null || digest.isEmpty) return null;
    return digest.startsWith('sha256:')
        ? digest.substring(7).toLowerCase()
        : null;
  }

  /// Launches the downloaded installer and gracefully exits the current app
  Future<void> launchInstallerAndExit() async {
    if (_downloadedFilePath == null || !File(_downloadedFilePath!).existsSync()) {
      _errorMessage = '未找到下载的安装程序文件';
      notifyListeners();
      return;
    }

    try {
      // Defense in depth behind the digest gate: refuse to launch an
      // installer whose Authenticode signature is present but invalid.
      // Unsigned builds (current releases) still pass.
      final sig = await Process.run('powershell', [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        '(Get-AuthenticodeSignature -FilePath "$_downloadedFilePath").Status',
      ]);
      final status = sig.stdout.toString().trim();
      const bad = {'HashMismatch', 'EmbedError', 'Corrupt'};
      if (bad.contains(status)) {
        _errorMessage = '安装包签名无效（$status），已停止安装';
        _downloadStatus = DownloadStatus.failed;
        notifyListeners();
        return;
      }

      // Execute the downloaded installer detached
      await Process.start(
        _downloadedFilePath!,
        [],
        mode: ProcessStartMode.detached,
      );

      // Cleanly exit current application so installer can overwrite binaries
      exit(0);
    } catch (e) {
      debugPrint('[UpdateProvider] Failed to launch installer: $e');
      _errorMessage = '启动安装包失败: $e';
      notifyListeners();
    }
  }

  /// Skip notification for current update version
  void skipCurrentVersion() {
    if (_updateInfo != null) {
      _settings = _settings.copyWith(skippedVersion: _updateInfo!.version);
      _saveSettings();
      _isDismissedForSession = true;
      notifyListeners();
    }
  }

  /// Clear skipped version to resume alerts
  void clearSkippedVersion() {
    _settings = _settings.copyWith(clearSkippedVersion: true);
    _saveSettings();
    _isDismissedForSession = false;
    notifyListeners();
  }

  /// Dismiss notification banner for current app launch session
  void dismissForSession() {
    _isDismissedForSession = true;
    notifyListeners();
  }

  /// Toggle auto-check setting
  void setAutoCheck(bool enabled) {
    _settings = _settings.copyWith(autoCheckEnabled: enabled);
    _saveSettings();
    notifyListeners();
  }

  /// Update check interval
  void setCheckInterval(UpdateCheckInterval interval) {
    _settings = _settings.copyWith(checkInterval: interval);
    _saveSettings();
    notifyListeners();
  }

  /// Toggle pre-release channel
  void setIncludePrerelease(bool include) {
    _settings = _settings.copyWith(includePrerelease: include);
    _saveSettings();
    notifyListeners();
  }

  /// Open release web page in default system browser
  static Future<void> openInBrowser(String url) async {
    try {
      if (Platform.isWindows) {
        await Process.run('cmd', ['/c', 'start', '', url]);
      } else {
        await Process.run('open', [url]);
      }
    } catch (e) {
      debugPrint('Failed to open browser URL: $e');
    }
  }
}
