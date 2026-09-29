import 'semver.dart';

/// Single asset attached to a release (e.g. Setup .exe or Portable .zip)
class ReleaseAsset {
  final String name;
  final int size;
  final String downloadUrl;
  final String contentType;

  const ReleaseAsset({
    required this.name,
    required this.size,
    required this.downloadUrl,
    required this.contentType,
  });

  factory ReleaseAsset.fromJson(Map<String, dynamic> json) {
    return ReleaseAsset(
      name: json['name'] as String? ?? '',
      size: (json['size'] as num?)?.toInt() ?? 0,
      downloadUrl: json['browser_download_url'] as String? ?? '',
      contentType: json['content_type'] as String? ?? '',
    );
  }

  Map<String, dynamic> toJson() => {
        'name': name,
        'size': size,
        'browser_download_url': downloadUrl,
        'content_type': contentType,
      };

  bool get isSetupInstaller {
    final lower = name.toLowerCase();
    return lower.endsWith('.exe') && lower.contains('setup');
  }

  bool get isPortableZip {
    final lower = name.toLowerCase();
    return lower.endsWith('.zip');
  }

  String get formattedSize {
    if (size <= 0) return '未知大小';
    final mb = size / (1024 * 1024);
    if (mb >= 1.0) {
      return '${mb.toStringAsFixed(1)} MB';
    }
    final kb = size / 1024;
    return '${kb.toStringAsFixed(1)} KB';
  }
}

/// Metadata and asset details of an available software update.
class UpdateInfo {
  final String version;
  final String tagName;
  final String title;
  final String releaseNotes;
  final DateTime publishedAt;
  final bool isPrerelease;
  final String htmlUrl;
  final ReleaseAsset? setupAsset;
  final ReleaseAsset? portableAsset;

  const UpdateInfo({
    required this.version,
    required this.tagName,
    required this.title,
    required this.releaseNotes,
    required this.publishedAt,
    required this.isPrerelease,
    required this.htmlUrl,
    this.setupAsset,
    this.portableAsset,
  });

  factory UpdateInfo.fromGitHubJson(Map<String, dynamic> json) {
    final tagName = json['tag_name'] as String? ?? '';
    final semVer = SemVer.tryParse(tagName);
    final versionStr = semVer != null
        ? '${semVer.major}.${semVer.minor}.${semVer.patch}${semVer.isPreRelease ? '-${semVer.preRelease.join('.')}' : ''}'
        : tagName.replaceFirst(RegExp(r'^[vV]'), '');

    final title = json['name'] as String? ?? tagName;
    final body = json['body'] as String? ?? '';
    final htmlUrl = json['html_url'] as String? ?? '';
    final isPrerelease = json['prerelease'] as bool? ?? false;

    DateTime pubDate = DateTime.now();
    if (json['published_at'] != null) {
      try {
        pubDate = DateTime.parse(json['published_at'] as String).toLocal();
      } catch (_) {}
    }

    ReleaseAsset? setup;
    ReleaseAsset? portable;

    if (json['assets'] is List) {
      final assetsList = json['assets'] as List;
      for (final item in assetsList) {
        if (item is Map<String, dynamic>) {
          final asset = ReleaseAsset.fromJson(item);
          if (asset.isSetupInstaller && setup == null) {
            setup = asset;
          } else if (asset.isPortableZip && portable == null) {
            portable = asset;
          }
        }
      }
    }

    return UpdateInfo(
      version: versionStr,
      tagName: tagName,
      title: title,
      releaseNotes: body,
      publishedAt: pubDate,
      isPrerelease: isPrerelease,
      htmlUrl: htmlUrl,
      setupAsset: setup,
      portableAsset: portable,
    );
  }

  /// Returns preferred asset for one-click in-app update (Setup installer preferred)
  ReleaseAsset? get primaryAsset => setupAsset ?? portableAsset;
}

/// Interval frequency for background auto-updates check.
enum UpdateCheckInterval {
  onStartup('每次启动时', 0),
  daily('每天一次 (24h)', 24),
  weekly('每周一次 (7天)', 168),
  manual('仅手动检查', -1);

  final String label;
  final int hours;
  const UpdateCheckInterval(this.label, this.hours);

  static UpdateCheckInterval fromString(String? val) {
    return UpdateCheckInterval.values.firstWhere(
      (e) => e.name == val,
      orElse: () => UpdateCheckInterval.daily,
    );
  }
}

/// Persistent user settings for update checks.
class UpdateSettings {
  final bool autoCheckEnabled;
  final UpdateCheckInterval checkInterval;
  final bool includePrerelease;
  final String? skippedVersion;
  final DateTime? lastCheckTime;

  const UpdateSettings({
    this.autoCheckEnabled = true,
    this.checkInterval = UpdateCheckInterval.daily,
    this.includePrerelease = false,
    this.skippedVersion,
    this.lastCheckTime,
  });

  UpdateSettings copyWith({
    bool? autoCheckEnabled,
    UpdateCheckInterval? checkInterval,
    bool? includePrerelease,
    String? skippedVersion,
    bool clearSkippedVersion = false,
    DateTime? lastCheckTime,
  }) {
    return UpdateSettings(
      autoCheckEnabled: autoCheckEnabled ?? this.autoCheckEnabled,
      checkInterval: checkInterval ?? this.checkInterval,
      includePrerelease: includePrerelease ?? this.includePrerelease,
      skippedVersion: clearSkippedVersion ? null : (skippedVersion ?? this.skippedVersion),
      lastCheckTime: lastCheckTime ?? this.lastCheckTime,
    );
  }

  Map<String, dynamic> toJson() => {
        'auto_check_enabled': autoCheckEnabled,
        'check_interval': checkInterval.name,
        'include_prerelease': includePrerelease,
        'skipped_version': skippedVersion,
        'last_check_time': lastCheckTime?.toIso8601String(),
      };

  factory UpdateSettings.fromJson(Map<String, dynamic> json) {
    DateTime? lastCheck;
    if (json['last_check_time'] != null) {
      try {
        lastCheck = DateTime.parse(json['last_check_time'] as String).toLocal();
      } catch (_) {}
    }

    return UpdateSettings(
      autoCheckEnabled: json['auto_check_enabled'] as bool? ?? true,
      checkInterval: UpdateCheckInterval.fromString(json['check_interval'] as String?),
      includePrerelease: json['include_prerelease'] as bool? ?? false,
      skippedVersion: json['skipped_version'] as String?,
      lastCheckTime: lastCheck,
    );
  }
}
