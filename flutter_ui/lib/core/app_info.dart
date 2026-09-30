/// Application information and release repository constants.
class AppInfo {
  static const String appName = 'GlobalTokenTracker++';
  static const String appShortName = 'GlobalTokenTrackerPP';
  static const String currentVersion = '1.4.0';
  static const String currentBuild = '6';
  static const String platform = 'Windows';
  static const String architecture = 'x64';

  /// GitHub Repository details
  static const String githubOwner = 'LiJiaHua1024';
  static const String githubRepo = 'GlobalTokenTrackerPP';
  static const String repoFullName = '$githubOwner/$githubRepo';

  /// GitHub API endpoints
  static const String releasesApiUrl =
      'https://api.github.com/repos/$githubOwner/$githubRepo/releases';
  static const String releasesLatestApiUrl =
      'https://api.github.com/repos/$githubOwner/$githubRepo/releases/latest';

  /// Web release page
  static const String releasesWebUrl =
      'https://github.com/$githubOwner/$githubRepo/releases';

  /// User Agent for GitHub API requests (GitHub requires a valid User-Agent)
  static const String userAgent =
      '$appShortName/$currentVersion ($platform; $architecture)';
}
