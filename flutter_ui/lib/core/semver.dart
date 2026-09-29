/// Semantic Versioning 2.0 parser and comparator.
///
/// Implements standard SemVer comparison logic, correctly handling
/// major, minor, patch numbers, and pre-release identifiers (alpha, beta, rc, etc.).
class SemVer implements Comparable<SemVer> {
  final int major;
  final int minor;
  final int patch;
  final List<String> preRelease;
  final String? buildMetadata;
  final String raw;

  const SemVer({
    required this.major,
    required this.minor,
    required this.patch,
    this.preRelease = const [],
    this.buildMetadata,
    required this.raw,
  });

  /// Parse a version string, gracefully stripping optional 'v' or 'V' prefixes.
  /// Returns null if format is not a recognizable semantic version.
  static SemVer? tryParse(String? input) {
    if (input == null) return null;
    final trimmed = input.trim();
    if (trimmed.isEmpty) return null;

    // Strip leading 'v' or 'V'
    final clean = trimmed.startsWith(RegExp(r'^[vV]')) ? trimmed.substring(1) : trimmed;

    // Split build metadata (+...)
    String versionAndPre = clean;
    String? buildMeta;
    final plusIndex = clean.indexOf('+');
    if (plusIndex != -1) {
      versionAndPre = clean.substring(0, plusIndex);
      buildMeta = clean.substring(plusIndex + 1);
    }

    // Split pre-release (-...)
    String coreVersion = versionAndPre;
    List<String> preReleaseParts = const [];
    final dashIndex = versionAndPre.indexOf('-');
    if (dashIndex != -1) {
      coreVersion = versionAndPre.substring(0, dashIndex);
      final preStr = versionAndPre.substring(dashIndex + 1);
      if (preStr.isNotEmpty) {
        preReleaseParts = preStr.split('.');
      }
    }

    // Parse core version digits
    final coreParts = coreVersion.split('.');
    if (coreParts.isEmpty || coreParts.length > 3) {
      return null;
    }

    final major = int.tryParse(coreParts[0]);
    if (major == null || major < 0) return null;

    final minor = coreParts.length > 1 ? (int.tryParse(coreParts[1]) ?? 0) : 0;
    if (minor < 0) return null;

    final patch = coreParts.length > 2 ? (int.tryParse(coreParts[2]) ?? 0) : 0;
    if (patch < 0) return null;

    return SemVer(
      major: major,
      minor: minor,
      patch: patch,
      preRelease: preReleaseParts,
      buildMetadata: buildMeta,
      raw: trimmed,
    );
  }

  /// Convenience utility to check if [remoteVersion] is strictly newer than [localVersion].
  static bool isNewer(String remoteVersion, String localVersion) {
    final remote = tryParse(remoteVersion);
    final local = tryParse(localVersion);
    if (remote == null || local == null) return false;
    return remote > local;
  }

  /// Whether this version is a pre-release version.
  bool get isPreRelease => preRelease.isNotEmpty;

  @override
  int compareTo(SemVer other) {
    // 1. Compare Major
    if (major != other.major) return major.compareTo(other.major);

    // 2. Compare Minor
    if (minor != other.minor) return minor.compareTo(other.minor);

    // 3. Compare Patch
    if (patch != other.patch) return patch.compareTo(other.patch);

    // 4. Compare Pre-release
    // A normal version has HIGHER precedence than a pre-release version
    // e.g. 1.0.0 > 1.0.0-alpha.1
    if (preRelease.isEmpty && other.preRelease.isNotEmpty) return 1;
    if (preRelease.isNotEmpty && other.preRelease.isEmpty) return -1;
    if (preRelease.isEmpty && other.preRelease.isEmpty) return 0;

    // Both are pre-release: compare parts
    final minLen = preRelease.length < other.preRelease.length
        ? preRelease.length
        : other.preRelease.length;

    for (int i = 0; i < minLen; i++) {
      final a = preRelease[i];
      final b = other.preRelease[i];

      final aNum = int.tryParse(a);
      final bNum = int.tryParse(b);

      if (aNum != null && bNum != null) {
        if (aNum != bNum) return aNum.compareTo(bNum);
      } else if (aNum != null && bNum == null) {
        // Numeric identifiers have lower precedence than non-numeric
        return -1;
      } else if (aNum == null && bNum != null) {
        return 1;
      } else {
        final cmp = a.compareTo(b);
        if (cmp != 0) return cmp;
      }
    }

    return preRelease.length.compareTo(other.preRelease.length);
  }

  bool operator >(SemVer other) => compareTo(other) > 0;
  bool operator <(SemVer other) => compareTo(other) < 0;
  bool operator >=(SemVer other) => compareTo(other) >= 0;
  bool operator <=(SemVer other) => compareTo(other) <= 0;

  @override
  bool operator ==(Object other) =>
      identical(this, other) || (other is SemVer && compareTo(other) == 0);

  @override
  int get hashCode => Object.hash(major, minor, patch, Object.hashAll(preRelease));

  @override
  String toString() => raw;
}
