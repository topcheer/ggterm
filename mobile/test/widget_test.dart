// Widget smoke test for the GGTerm connection screen.
//
// The full app (GGTermApp) instantiates SessionManager which performs
// dart:ffi lookups against the native Rust library.  That native lib is
// not available in the unit-test environment, so we test ConnectionScreen
// directly — it is a pure Flutter widget with no FFI dependency.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:ggterm_mobile/connection_screen.dart';

void main() {
  testWidgets('Connection screen shows title and Echo Test button',
      (WidgetTester tester) async {
    await tester.pumpWidget(
      MaterialApp(
        home: ConnectionScreen(
          onConnect: (_) async {},
          onEchoTest: () {},
        ),
      ),
    );

    // AppBar title
    expect(find.text('GGTerm — Connect'), findsOneWidget);

    // Echo Test button is visible
    expect(find.text('Echo Test (No SSH)'), findsOneWidget);
  });

  testWidgets('Tapping Connect triggers onConnect callback',
      (WidgetTester tester) async {
    ConnectionParams? captured;

    await tester.pumpWidget(
      MaterialApp(
        home: ConnectionScreen(
          onConnect: (params) async {
            captured = params;
          },
          onEchoTest: () {},
        ),
      ),
    );

    // Enter host and user, then tap Connect
    await tester.enterText(
      find.byType(TextField).at(0),
      'example.com',
    );
    await tester.enterText(
      find.byType(TextField).at(2),
      'root',
    );

    await tester.tap(find.text('Connect'));
    await tester.pumpAndSettle();

    expect(captured, isNotNull);
    expect(captured!.host, 'example.com');
    expect(captured!.username, 'root');
  });
}
