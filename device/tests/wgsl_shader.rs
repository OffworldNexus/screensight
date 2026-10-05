//! Offline WGSL validation for the vendored GPUI blade shaders.
//!
//! The panel's CRT screen transition lives in gpui's `shaders.wgsl`, which is
//! only compiled by the GPU at runtime. Parsing and validating it with `naga`
//! here means a broken shader fails the test suite instead of the panel on the
//! Pi. gpui's own build script already parses the file; this additionally runs
//! the full validator so type/layout errors are caught.

/// gpui's blade shader source, vendored under `vendor/gpui`.
const SHADER: &str = include_str!("../../vendor/gpui/src/platform/blade/shaders.wgsl");

#[test]
fn blade_shaders_parse_and_validate() {
    let module = naga::front::wgsl::parse_str(SHADER).expect("WGSL should parse");

    // gpui's shader declares resources without `@group`/`@binding`; blade binds
    // them through its own reflection layer, so skip only the binding checks and
    // still validate types, expressions and entry points.
    let mut flags = naga::valid::ValidationFlags::all();
    flags.remove(naga::valid::ValidationFlags::BINDINGS);

    let mut validator = naga::valid::Validator::new(flags, naga::valid::Capabilities::all());
    validator.validate(&module).expect("WGSL should validate");
}
