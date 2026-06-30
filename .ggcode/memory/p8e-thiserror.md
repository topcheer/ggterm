P8-E thiserror error type unification complete (commit 1a44e25):
- 7 error types migrated to #[derive(Error)]:
  - PtyError, AIError, RenderError, PluginError, ConfigError, GpuError, RenderFrameError
- thiserror = "2" workspace dep
- All #[from] auto-generated From impls replace manual ones
- 834 tests default, 987 tests with all features, 0 failures