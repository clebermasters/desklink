# JNI entry points are looked up by name from libdesklink_android.so.
-keep class com.desklink.android.NativeBridge { *; }
# Started by name through app_process from an adb shell.
-keep class com.desklink.android.shell.InputHelper { public static void main(java.lang.String[]); }
# Hidden framework methods reached by reflection inside the shell helper.
-dontwarn android.hardware.input.InputManagerGlobal
