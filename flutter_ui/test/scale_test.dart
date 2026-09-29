import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('UI scaling fits window exactly and caption buttons never overflow', (tester) async {
    final binding = tester.binding;

    for (final s in [0.85, 1.0, 1.15, 1.25, 1.40]) {
      for (final windowSize in [const Size(1300, 820), const Size(960, 640), const Size(1920, 1080)]) {
        await binding.setSurfaceSize(windowSize);
        bool closeBtnTapped = false;
        bool minBtnTapped = false;

        await tester.pumpWidget(
          MaterialApp(
            builder: (context, child) {
              if (s == 1.0 || child == null) {
                return child ?? const SizedBox.shrink();
              }
              final mq = MediaQuery.of(context);
              final scaledW = mq.size.width / s;
              final scaledH = mq.size.height / s;
              return MediaQuery(
                data: mq.copyWith(
                  size: Size(scaledW, scaledH),
                ),
                child: FittedBox(
                  fit: BoxFit.fill,
                  alignment: Alignment.topLeft,
                  child: SizedBox(
                    width: scaledW,
                    height: scaledH,
                    child: child,
                  ),
                ),
              );
            },
            home: Scaffold(
              body: Column(
                children: [
                  // Mock Title Bar with 3 buttons
                  Container(
                    height: 38,
                    color: Colors.grey,
                    child: Row(
                      children: [
                        const Padding(
                          padding: EdgeInsets.symmetric(horizontal: 12),
                          child: Text('GlobalTokenTracker++'),
                        ),
                        const Expanded(child: SizedBox()),
                        GestureDetector(
                          key: const Key('min_button'),
                          onTap: () => minBtnTapped = true,
                          child: Container(width: 44, height: 38, color: Colors.blue),
                        ),
                        Container(
                          key: const Key('max_button'),
                          width: 44,
                          height: 38,
                          color: Colors.green,
                        ),
                        GestureDetector(
                          key: const Key('close_button'),
                          onTap: () => closeBtnTapped = true,
                          child: Container(width: 44, height: 38, color: Colors.red),
                        ),
                      ],
                    ),
                  ),
                  Expanded(
                    child: Row(
                      children: [
                        const SizedBox(width: 72, child: Text('Nav')),
                        Expanded(
                          child: Container(
                            key: const Key('main_content'),
                            color: Colors.white,
                          ),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        );

        final closeBox = tester.getRect(find.byKey(const Key('close_button')));
        final minBox = tester.getRect(find.byKey(const Key('min_button')));
        final mainContentBox = tester.getRect(find.byKey(const Key('main_content')));

        // 1. Close button right edge must align exactly with the window width
        expect(closeBox.right, closeTo(windowSize.width, 0.01),
            reason: 'Scale $s at window width ${windowSize.width} must align with right window edge');
        expect(closeBox.left, lessThan(windowSize.width));

        // 2. Window control buttons must be fully inside the visible window area
        expect(minBox.left, greaterThanOrEqualTo(0));
        expect(minBox.right, lessThanOrEqualTo(windowSize.width));

        // 3. Main content must not overflow the window boundary
        expect(mainContentBox.right, lessThanOrEqualTo(windowSize.width + 0.01));
        expect(mainContentBox.bottom, closeTo(windowSize.height, 0.01));

        // 4. Hit tests at scaled coordinates must properly trigger button actions
        await tester.tap(find.byKey(const Key('close_button')));
        expect(closeBtnTapped, isTrue,
            reason: 'Tapping close button at scale $s, window $windowSize must succeed');

        await tester.tap(find.byKey(const Key('min_button')));
        expect(minBtnTapped, isTrue,
            reason: 'Tapping min button at scale $s, window $windowSize must succeed');
      }
    }
  });
}
