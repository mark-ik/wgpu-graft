// This patched crate always emits the `#[link(name = "fontconfig")] extern
// "C" { ... }` block (so direct callers like servo-fonts work), so libfontconfig
// must be present at link time. Always run pkg-config to record the link line —
// upstream's conditional `cfg!(feature = "dlopen")` gating is intentionally
// dropped.
fn main() {
    pkg_config::find_library("fontconfig").unwrap();
}
