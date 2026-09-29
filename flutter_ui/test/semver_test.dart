import 'package:flutter_test/flutter_test.dart';
import 'package:globaltokentracker_ui/core/semver.dart';

void main() {
  group('SemVer parser and comparison tests', () {
    test('Basic parsing of version strings with and without v prefix', () {
      final v1 = SemVer.tryParse('1.0.0');
      expect(v1, isNotNull);
      expect(v1!.major, 1);
      expect(v1.minor, 0);
      expect(v1.patch, 0);
      expect(v1.isPreRelease, isFalse);

      final v2 = SemVer.tryParse('v1.2.3');
      expect(v2, isNotNull);
      expect(v2!.major, 1);
      expect(v2.minor, 2);
      expect(v2.patch, 3);
    });

    test('Pre-release versions', () {
      final v = SemVer.tryParse('v1.0.0-alpha.1');
      expect(v, isNotNull);
      expect(v!.major, 1);
      expect(v.isPreRelease, isTrue);
      expect(v.preRelease, ['alpha', '1']);
    });

    test('Comparison logic', () {
      expect(SemVer.isNewer('1.0.1', '1.0.0'), isTrue);
      expect(SemVer.isNewer('1.1.0', '1.0.9'), isTrue);
      expect(SemVer.isNewer('2.0.0', '1.9.9'), isTrue);
      expect(SemVer.isNewer('1.0.0', '1.0.0'), isFalse);
      expect(SemVer.isNewer('0.9.9', '1.0.0'), isFalse);

      // Normal release is newer than pre-release of the same version
      expect(SemVer.isNewer('1.0.0', '1.0.0-alpha.1'), isTrue);
      expect(SemVer.isNewer('1.0.0-alpha.1', '1.0.0'), isFalse);

      // Pre-release comparison
      expect(SemVer.isNewer('1.0.0-alpha.2', '1.0.0-alpha.1'), isTrue);
      expect(SemVer.isNewer('1.0.0-beta.1', '1.0.0-alpha.2'), isTrue);
      expect(SemVer.isNewer('1.0.0-rc.1', '1.0.0-beta.5'), isTrue);
      expect(SemVer.isNewer('1.0.1-alpha.1', '1.0.0'), isTrue);
    });
  });
}
