use std::env;
use std::fs::File;
use std::path::Path;

use gl_generator::{Api, Fallbacks, Profile, Registry, StructGenerator};

fn main() {
    // The GL bindings back the `gl` feature's raw_gl path only. Non-GL consumers
    // (wgpu-weld, wgpu-scry's shared-texture import) build grafting without `gl`,
    // so skip the codegen entirely — `lib.rs`'s `gl_bindings` module is gated the
    // same way and never `include!`s the missing file.
    if env::var_os("CARGO_FEATURE_GL").is_none() {
        return;
    }

    let out = env::var("OUT_DIR").unwrap();
    let out = Path::new(&out);

    let mut file = File::create(out.join("gl_bindings.rs")).unwrap();

    Registry::new(
        Api::Gles2,
        (3, 0),
        Profile::Core,
        Fallbacks::All,
        [
            "GL_EXT_memory_object",
            "GL_EXT_memory_object_fd",
            "GL_EXT_memory_object_win32",
            "GL_EXT_semaphore",
            "GL_EXT_semaphore_fd",
            "GL_EXT_semaphore_win32",
        ],
    )
    .write_bindings(StructGenerator, &mut file)
    .unwrap();
}
