---
name: wgpu-graft project context
description: Workspace for embedding GL-rendered content (Servo via surfman) into host wgpu textures. Extracted from Slint, renamed from wgpu-gui-bridge 2026-05-05. Two lib crates plus Servo demos. Linux/Apple import paths implemented, Windows DX12 deferred. Architecturally complementary to WebRender wgpu backend work.
type: project
---

**wgpu-graft** at `c:\Users\mark_\Code\repos\wgpu-graft` is a Rust workspace for grafting an external GPU producer's texture (today: Servo via surfman/GL) onto host-owned wgpu textures.

**Naming:** Renamed from `wgpu-gui-bridge` on 2026-05-05. "Graft" carries the surgical/horticultural sense — joining an external GPU resource onto a wgpu host. Bare `graft` was already taken on crates.io (orbitinghail's storage engine), so the workspace and primary crate are namespaced as `wgpu-graft`.

**Why:** Servo currently renders via GL (surfman). Host apps increasingly use wgpu. The graft, derived from the Slint repo's Servo example, closes the gap with platform-specific import paths. Also applicable beyond Servo — potentially any GL-rendering app could use the raw path, which has been disambiguated from surfman.

**How to apply:** This project is complementary to ongoing WebRender wgpu backend work. In the short term, GL-interop is useful because Servo's GL path won't change immediately. Long term: when WebRender has a production wgpu backend, the interop either won't be needed or simplifies to same-device texture sharing (Phase 3 in the plan).

**Key architecture insight:** The GL import logic is currently coupled to surfman. Decoupling it (Phase 1) makes the graft usable by any GL producer. The build.rs already generates Windows GL extension bindings (`GL_EXT_memory_object_win32`) for the future Windows path.

**Platform paths:**
- Linux: GL FBO → Vulkan external memory FD → wgpu texture
- Apple: IOSurface → Metal texture → BGRA→RGBA normalization → wgpu texture
- Windows: API stubbed (`Dx12SharedTexture`), runtime not implemented

**wgpu version:** 29.0.1. Uses `wgpu-hal` for `create_texture_from_hal` (both Vulkan and Metal paths).
