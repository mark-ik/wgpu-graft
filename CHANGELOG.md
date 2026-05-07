# Changelog

All notable changes to this project will be documented here.

## [Unreleased]

### Added — `wgpu-native-texture-interop` 0.2.0

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
- Cargo features: added `Win32_Security` to the `windows` crate dep
  (required by `ID3D12Device::CreateSharedHandle`); added `MTLEvent` to
  `objc2-metal` (required by `newSharedEvent`)

### Added

- `README.md`: documented the branch policy for `main`, `latest-release`,
  `experimental`, and `servo-wgpu`, and clarified that `main` targets Servo
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
