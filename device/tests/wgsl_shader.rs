//! Offline WGSL validation for the pinned GPUI blade shaders.
//!
//! The panel's CRT screen transition lives in gpui's `shaders.wgsl`, which is
//! only compiled by the GPU at runtime. Parsing and validating it with `naga`
//! here means a broken shader fails the test suite instead of the panel on the
//! Pi.
//!
//! gpui now lives in a rev-pinned fork (see the root `Cargo.toml`), so the
//! shader is located through cargo's git checkout at runtime. A build without
//! the `gui` feature never fetches gpui, in which case the test skips; a `gui`
//! build (including CI) does fetch it and the shader is validated for real.

use std::path::PathBuf;

/// Read gpui's blade shader from cargo's git checkout of the pinned fork.
fn pinned_shader() -> Option<String> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))?;
    for checkout in std::fs::read_dir(cargo_home.join("git").join("checkouts"))
        .ok()?
        .flatten()
    {
        if !checkout.file_name().to_string_lossy().starts_with("gpui-") {
            continue;
        }
        let Ok(revisions) = std::fs::read_dir(checkout.path()) else {
            continue;
        };
        for revision in revisions.flatten() {
            let shader = revision.path().join("src/platform/blade/shaders.wgsl");
            if let Ok(source) = std::fs::read_to_string(&shader) {
                return Some(source);
            }
        }
    }
    None
}

#[test]
fn blade_shaders_parse_and_validate() {
    let Some(shader) = pinned_shader() else {
        eprintln!("gpui blade shaders not in the cargo checkout; skipping WGSL validation");
        return;
    };

    let module = naga::front::wgsl::parse_str(&shader).expect("WGSL should parse");

    // gpui's shader declares resources without `@group`/`@binding`; blade binds
    // them through its own reflection layer, so skip only the binding checks and
    // still validate types, expressions and entry points.
    let mut flags = naga::valid::ValidationFlags::all();
    flags.remove(naga::valid::ValidationFlags::BINDINGS);

    let mut validator = naga::valid::Validator::new(flags, naga::valid::Capabilities::all());
    validator.validate(&module).expect("WGSL should validate");
}
