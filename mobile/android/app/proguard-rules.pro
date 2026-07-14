# mobile_scanner — keep camera plugin classes
-keep class dev.steenbakker.mobile_scanner.** { *; }
-dontwarn dev.steenbakker.mobile_scanner.**

# Keep Flutter platform view and method channel infrastructure
-keep class io.flutter.** { *; }
-dontwarn io.flutter.**

# Keep AndroidX CameraX classes
-keep class androidx.camera.** { *; }
-dontwarn androidx.camera.**

# permission_handler
-keep class com.baseflow.permissionhandler.** { *; }
-dontwarn com.baseflow.permissionhandler.**
