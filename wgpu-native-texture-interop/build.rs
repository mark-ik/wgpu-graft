use std::env;
use std::fs::File;
use std::path::Path;

use gl_generator::{Api, Fallbacks, Profile, Registry, StructGenerator};

fn main() {
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
