# Changelog

All notable changes to this project will be documented here.

## [Unreleased]

### Added — `wgpu-native-texture-interop` 0.2.0

- `surfman_gl::windows_dx12_shared`: ANGLE D3D11 → wgpu DX12 zero-copy
  import path. Allocates an `ID3D11Texture2D` with
  `D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE` on
  ANGLE's own D3D11 device, wraps it as a transient EGL pbuffer surface
  for ANGLE/GL writes, and opens the same NT handle on the host wgpu
  DX12 device via `ID3D12Device::OpenSharedHandle`. Closes the gap
  where `raw_gl::dx12` could not service ANGLE-Servo (which lacks
  `GL_EXT_memory_object_win32`). Adapted from slint examples/servo
  (#11089). Size-dependent state is cached on `SurfmanFrameProducer`
  via `AngleDx12SharedCache` and reused across frames so the wgpu
  texture handle stays stable
- `surfman_gl::select_adapter_matching_surfman_luid`: Windows multi-GPU
  adapter selection helper that matches wgpu's adapter LUID to
  surfman's underlying D3D11 device LUID. On hosts with both an
  integrated and discrete GPU, wgpu's `request_adapter` and surfman's
  `Connection::create_adapter` may otherwise pick different drivers,
  silently breaking the shared-NT-handle interop. Adapted from slint
  examples/servo (#11439)
- `backend_name(&wgpu::Device) -> &'static str` and
  `print_wgpu_backend(&wgpu::Device)`: reports the active wgpu graphics
  backend in human-readable form for startup observability
- `Dx12FenceSynchronizer`: explicit `D3D12_FENCE_FLAG_SHARED` fence
  synchronizer for cross-API texture handoff. Creates a shared fence on
  the wgpu D3D12 device, exports an NT handle for D3D11/D3D12 producers,
  and queues `ID3D12CommandQueue::Wait` on the wgpu queue before each
  consumer submit
- `VulkanSemaphoreSynchronizer`: external `VkSemaphore` fd-based
  synchronizer for the WPE DMABUF protocol on Linux. Imports a per-frame
  semaphore fd into a persistent `VkSemaphore` with `TEMPORARY` flag and
  issues a standalone wait submit on the wgpu Vulkan queue
- `MetalSharedEventSynchronizer`: precautionary `MTLSharedEvent`
  synchronizer for Apple platforms; CPU-side wait via
  `waitUntilSignaledValue:timeoutMS:`. Not required for correctness on
  Apple silicon (IOSurface coherence is implicit) but provides the API
  anchor for a future GPU-side wait once `wgpu-hal::metal::Queue`
  exposes its raw `MTLCommandQueue`
- `VulkanExternalImage` import path: DMABUF→`VkImage`→`wgpu::Texture` via
  `VK_KHR_external_memory_fd` + `VK_EXT_image_drm_format_modifier`
  (Linux only). Replaces the prior
  `Unsupported(NativeImportNotYetImplemented)` arm with a real import
  for WPE-class DMABUF producers
- `VulkanExternalImage` fields for DMABUF and semaphore handoff:
  `dmabuf_fd`, `dmabuf_offset`, `dmabuf_stride`, `drm_modifier`,
  `wait_semaphore_fd`

### Changed — `wgpu-native-texture-interop` 0.2.0

- `CapabilityMatrix::vulkan_external_image`: now reports `Supported`
  on Linux + Vulkan host backend (was
  `Unsupported(NativeImportNotYetImplemented)`)
- `InteropBackend::Dx12` doc string updated to reflect that GL→DX12
  import is now supported on ANGLE-backed surfman via
  `surfman_gl::windows_dx12_shared`
- Cargo features: added `Win32_Security` and `Win32_Graphics_Direct3D11`
  to the `windows` crate dep (required by the new shared-D3D11 path);
  added `sm-angle-default` to surfman (required for ANGLE-specific
  `Device::create_surface_texture_from_texture`); added `wio = "0.2"`
  for the surfman ANGLE method's `ComPtr` parameter; added `MTLEvent`
  to `objc2-metal` (required by `newSharedEvent`)
- Surfman rebind errors are now propagated through the Linux Vulkan,
  Windows Vulkan, Windows DX12, and Apple Metal import paths (was
  silently swallowed via `let _ = ...`). Both the import and rebind
  attempt run; whichever fails surfaces (preferring the import error
  if both fail). Adapted from slint examples/servo (#11497)

### Demo changes

- `demo-servo-winit`: switched the Windows wgpu instance from
  `VULKAN | DX12` to forcing DX12 by default so the new
  `surfman_gl::windows_dx12_shared` path is the exercised default.
  `WGPU_BACKEND=vulkan` still selects the legacy ANGLE-D3D11 KMT →
  Vulkan path. Calls `print_wgpu_backend` on startup.

### Added

- `README.md`: documented the branch policy for `main`, `latest-release`,
  and `experimental`, and clarified that `main` targets Servo
  `v0.1.x` LTS
- `demo-servo-xilem`: Servo embedded in Xilem 0.4 with URL bar, CPU readback,
  and full input forwarding (mouse, scroll, keyboard)
- `demo-servo-iced`: Servo embedded in iced 0.14 with URL bar, CPU readback,
  flicker-free GPU upload via `image::allocate()`, and full input forwarding
- `demo-servo-gpui`: Servo embedded in GPUI 0.2 (Zed's framework) with URL bar,
  RGBA→BGRA conversion, `request_animation_frame()` render loop, and full input
  forwarding including custom key mapping
- `demo-servo-winit`: added mouse, scroll, and keyboard input forwarding to
  Servo; pages are now fully interactive (links, scrolling, text input)
- `rust-toolchain.toml`: pin workspace to Rust 1.92.0 (required by wgpu 28)
- `patches/gpui`: local gpui fork with taffy `=0.9.0` → `0.9.2` for
  compatibility with servo-layout
- `patches/serde_fmt`: local serde_fmt fork removing ambiguous `From` impl
  that breaks stylo's `ToCss` derive on Rust 1.92
- `wgpu-native-texture-interop`: public API doc comments on all major types
  (`InteropBackend`, `CapabilityMatrix`, `NativeFrame`, `ImportOptions`, etc.)
- `wgpu-native-texture-interop`: `#[non_exhaustive]` on `NativeFrame`,
  `NativeFrameKind`, `InteropBackend`, `SyncMechanism`, `InteropError`, and
  `UnsupportedReason` to protect downstream users from semver breaks
- `wgpu-native-texture-interop`, `servo-wgpu-interop-adapter`: crate-level
  `#![doc = include_str!("../README.md")]` so docs.rs renders the README

### Fixed

- `raw_gl/linux.rs`, `raw_gl/windows.rs`: Vulkan memory allocation now
  correctly queries `get_physical_device_memory_properties` and selects a
  `DEVICE_LOCAL` memory type index compatible with the image's
  `memory_type_bits`, rather than unconditionally using index 0

## [0.1.0] — Initial release

- GL→wgpu texture interop for Linux/Android (Vulkan opaque FD) and Apple
  (IOSurface→Metal)
- Windows Vulkan path (opaque Win32 NT handle) — builds and runs; depends on
  driver support for `VK_KHR_external_memory_win32` under WGL/EGL
- `wgpu-native-texture-interop`: core library with trait-based API
- `servo-wgpu-interop-adapter`: Servo `RenderingContext` integration
- `demo-raw-gl`: standalone glutin+glow FBO → wgpu demo (no Servo required)
- `demo-servo-winit`: full Servo + winit + wgpu reference application
