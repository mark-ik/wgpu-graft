//! Cross-API adapter selection for surfman + wgpu.
//!
//! On multi-GPU Windows hosts (e.g. Intel iGPU + Arc/NVIDIA dGPU), wgpu's
//! `request_adapter` and surfman's `Connection::create_adapter` may pick
//! different physical GPUs. When the wgpu device runs on the dGPU but
//! surfman/ANGLE binds the iGPU, the shared-NT-handle interop in
//! [`super::windows_dx12_shared`] silently fails or produces incorrect
//! results because the two halves of the import are on different drivers.
//!
//! Adapted from slint-ui/slint examples/servo (PR #11439).

#[cfg(target_os = "windows")]
use windows::Win32::Graphics::{Direct3D11::ID3D11Device, Dxgi};
#[cfg(target_os = "windows")]
use windows::core::{IUnknown, Interface};

use crate::InteropError;

/// Pick a surfman adapter that resolves to the same physical GPU as the
/// supplied wgpu device. Windows + DX12 only.
///
/// Iterates surfman's adapter presets (hardware → low-power → default),
/// instantiates each one to extract its underlying D3D11 device LUID, and
/// returns the first whose LUID matches the wgpu device's adapter LUID. The
/// LUID comparison is what makes shared NT-handle imports work — surfman's
/// pbuffer texture and the wgpu-imported DX12 resource must live on the
/// same driver.
///
/// On non-Windows targets, this is a no-op-shaped wrapper that just calls
/// [`surfman::Connection::create_adapter`]; the returned adapter is whatever
/// surfman would have picked anyway.
///
/// # Errors
///
/// - [`InteropError::BackendMismatch`] if `wgpu_device` is not running on
///   DX12 (Windows). The shared-handle path requires DX12 on the host side.
/// - [`InteropError::Surfman`] if surfman could not produce any adapter.
/// - [`InteropError::Dx12`] if no surfman adapter's LUID matched the wgpu
///   device's LUID (e.g. wgpu picked a discrete GPU but only an integrated
///   surfman adapter is reachable).
pub fn select_adapter_matching_surfman_luid(
    connection: &surfman::Connection,
    wgpu_device: &wgpu::Device,
) -> Result<surfman::Adapter, InteropError> {
    #[cfg(not(target_os = "windows"))]
    {
        // No LUID concept on Linux/macOS; the surfman default adapter is fine.
        let _ = wgpu_device;
        return connection
            .create_adapter()
            .map_err(|err| InteropError::Surfman(format!("create_adapter failed: {err:?}")));
    }

    #[cfg(target_os = "windows")]
    {
        let wgpu_luid = unsafe {
            wgpu_device
                .as_hal::<wgpu::wgc::api::Dx12>()
                .ok_or(InteropError::BackendMismatch {
                    expected: "Dx12",
                    actual: "non-Dx12",
                })?
                .raw_device()
                .GetAdapterLuid()
        };

        for create_adapter_fn in [
            surfman::Connection::create_hardware_adapter
                as fn(&surfman::Connection) -> Result<surfman::Adapter, surfman::Error>,
            surfman::Connection::create_low_power_adapter,
            surfman::Connection::create_adapter,
        ] {
            let Ok(surfman_adapter) = create_adapter_fn(connection) else {
                continue;
            };
            let Ok(temp_device) = connection.create_device(&surfman_adapter) else {
                continue;
            };
            let d3d11_device_ptr = temp_device.native_device().d3d11_device;
            if d3d11_device_ptr.is_null() {
                continue;
            }

            let surfman_luid = unsafe {
                let d3d11_device: ID3D11Device =
                    match IUnknown::from_raw(d3d11_device_ptr as *mut _).cast() {
                        Ok(dev) => dev,
                        Err(_) => continue,
                    };
                let dxgi_device = match d3d11_device.cast::<Dxgi::IDXGIDevice>() {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let dxgi_adapter = match dxgi_device.GetAdapter() {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                let desc = match dxgi_adapter.GetDesc() {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                desc.AdapterLuid
            };

            if surfman_luid.HighPart == wgpu_luid.HighPart
                && surfman_luid.LowPart == wgpu_luid.LowPart
            {
                return Ok(surfman_adapter);
            }
        }

        Err(InteropError::Dx12(
            "no surfman adapter LUID matches the wgpu DX12 device LUID — \
             multi-GPU host with surfman pinned to a different driver?"
                .into(),
        ))
    }
}
