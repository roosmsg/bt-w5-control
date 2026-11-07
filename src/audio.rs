//! Fixed audio defaults for the BT-W5 endpoints:
//! - Creative's APO bypassed (Windows "audio enhancements" off), which is "Effects Off" and EQ off,
//! - playback as plain stereo,
//! - headphones 24-bit / 96 kHz, microphone the highest format the device supports.
//!
//! Formats and endpoint properties are written through the same undocumented
//! `IPolicyConfig` (Vista variant) COM object the Creative App uses.

use std::ffi::c_void;
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_EXCLUSIVE, DEVICE_STATE_ACTIVE, EDataFlow, IAudioClient, IMMDevice,
    IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, WAVEFORMATEX, eAll, eCapture, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::Win32::System::Variant::VT_UI4;
use windows::core::{GUID, Interface, PCWSTR};

fn propvariant_u32(value: u32) -> PROPVARIANT {
    PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_UI4,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { ulVal: value },
            }),
        },
    }
}

use policy_config::{CLSID_POLICY_CONFIG_VISTA, IPolicyConfigVista};

/// The undocumented `IPolicyConfig` Vista variant. Every slot must stay in vtable order.
#[allow(non_snake_case, dead_code)]
mod policy_config {
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::{ERole, WAVEFORMATEX};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::core::{BOOL, GUID, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR};

    pub const CLSID_POLICY_CONFIG_VISTA: GUID =
        GUID::from_u128(0x294935ce_f637_4e7c_a41b_ab255460b862);

    #[windows_core::interface("568b9108-44bf-40b4-9006-86afe5b5a620")]
    pub unsafe trait IPolicyConfigVista: IUnknown {
        pub fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut WAVEFORMATEX) -> HRESULT;
        pub fn GetDeviceFormat(
            &self,
            device: PCWSTR,
            default: BOOL,
            format: *mut *mut WAVEFORMATEX,
        ) -> HRESULT;
        pub fn SetDeviceFormat(
            &self,
            device: PCWSTR,
            endpoint_format: *const WAVEFORMATEX,
            mix_format: *const WAVEFORMATEX,
        ) -> HRESULT;
        pub fn GetProcessingPeriod(
            &self,
            device: PCWSTR,
            default: BOOL,
            default_period: *mut i64,
            minimum_period: *mut i64,
        ) -> HRESULT;
        pub fn SetProcessingPeriod(&self, device: PCWSTR, period: *const i64) -> HRESULT;
        pub fn GetShareMode(&self, device: PCWSTR, mode: *mut i32) -> HRESULT;
        pub fn SetShareMode(&self, device: PCWSTR, mode: *const i32) -> HRESULT;
        pub fn GetPropertyValue(
            &self,
            device: PCWSTR,
            fx_store: BOOL,
            key: *const PROPERTYKEY,
            value: *mut PROPVARIANT,
        ) -> HRESULT;
        pub fn SetPropertyValue(
            &self,
            device: PCWSTR,
            fx_store: BOOL,
            key: *const PROPERTYKEY,
            value: *const PROPVARIANT,
        ) -> HRESULT;
        pub fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        pub fn SetEndpointVisibility(&self, device: PCWSTR, visible: BOOL) -> HRESULT;
    }
}

const fn key(fmtid: u128, pid: u32) -> PROPERTYKEY {
    PROPERTYKEY {
        fmtid: GUID::from_u128(fmtid),
        pid,
    }
}

/// PKEY_AudioEndpoint_Disable_SysFx: 1 bypasses all endpoint effects (the Creative APO).
const PKEY_DISABLE_SYSFX: PROPERTYKEY = key(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e, 5);
/// PKEY_AudioEndpoint_PhysicalSpeakers: speaker mask, 3 = front left + right.
const PKEY_PHYSICAL_SPEAKERS: PROPERTYKEY = key(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e, 3);
/// Endpoint's device-node string, e.g. `{1}.USB\VID_041E&PID_3130&MI_02\...`.
const PKEY_ENDPOINT_DEVNODE: PROPERTYKEY = key(0xb3f8fa53_0004_438e_9003_51a46e139bfc, 2);
/// PKEY_Device_FriendlyName.
const PKEY_FRIENDLY_NAME: PROPERTYKEY = key(0xa45c254e_df1c_4efd_8020_67d146a850e0, 14);
/// Driver info `inf:hash:section:version:hwid`, which the Creative App shows as driver version.
const PKEY_DRIVER_INFO: PROPERTYKEY = key(0x83da6326_97a6_4088_9453_a1923f573b29, 3);

const KSDATAFORMAT_SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const SPEAKER_STEREO: u32 = 0x3;

const HEADPHONE_FORMAT: Format = Format {
    channels: 2,
    rate: 96_000,
    bits: 24,
};

#[repr(C, packed(1))]
#[derive(Clone, Copy)]
struct WaveFormatExtensible {
    tag: u16,
    channels: u16,
    rate: u32,
    avg_bytes: u32,
    block_align: u16,
    bits: u16,
    extra_size: u16,
    valid_bits: u16,
    channel_mask: u32,
    sub_format: GUID,
}

impl WaveFormatExtensible {
    fn pcm(format: Format, channel_mask: u32) -> Self {
        let block_align = format.channels * format.bits / 8;
        Self {
            tag: WAVE_FORMAT_EXTENSIBLE,
            channels: format.channels,
            rate: format.rate,
            avg_bytes: format.rate * u32::from(block_align),
            block_align,
            bits: format.bits,
            extra_size: 22,
            valid_bits: format.bits,
            channel_mask,
            sub_format: KSDATAFORMAT_SUBTYPE_PCM,
        }
    }

    fn as_wave(&self) -> *const WAVEFORMATEX {
        (self as *const Self).cast()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub headphones: Option<Format>,
    pub microphone: Option<Format>,
    /// `inf:hash:section:version:hwid` version field of the audio driver.
    pub driver_version: Option<String>,
    pub problems: Vec<String>,
}

struct Endpoint {
    id: Vec<u16>,
    device: IMMDevice,
    flow: EDataFlow,
    driver_info: Option<String>,
}

impl Endpoint {
    fn pcwstr(&self) -> PCWSTR {
        PCWSTR(self.id.as_ptr())
    }
}

/// Applies the fixed defaults to every active BT-W5 endpoint and reports the resulting formats.
/// With `apply == false` nothing is written and the current formats are reported.
/// Runs on its own thread because COM is initialised per thread.
pub fn apply_defaults(apply: bool) -> Summary {
    std::thread::spawn(move || unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let summary = apply_defaults_inner(apply);
        if init.is_ok() {
            CoUninitialize();
        }
        summary
    })
    .join()
    .unwrap_or_default()
}

/// Turns Windows audio enhancements back on for the BT-W5 endpoints (undoes the APO bypass).
/// Formats are left as they are. Used by `--uninstall`.
pub fn restore_enhancements() -> Vec<String> {
    std::thread::spawn(|| unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let lines = (|| -> windows::core::Result<Vec<String>> {
            let policy: IPolicyConfigVista =
                CoCreateInstance(&CLSID_POLICY_CONFIG_VISTA, None, CLSCTX_ALL)?;
            let mut lines = Vec::new();
            for endpoint in find_endpoints()? {
                let flow = if endpoint.flow == eRender {
                    "render"
                } else {
                    "capture"
                };
                let result = set_u32(&policy, &endpoint, &PKEY_DISABLE_SYSFX, 0);
                lines.push(format!("{flow}: enhancements on: {result:?}"));
            }
            Ok(lines)
        })()
        .unwrap_or_else(|e| vec![format!("error: {e}")]);
        if init.is_ok() {
            CoUninitialize();
        }
        lines
    })
    .join()
    .unwrap_or_default()
}

/// Read-only report of the BT-W5 endpoints, for `--probe`. Writes nothing.
pub fn inspect() -> Vec<String> {
    std::thread::spawn(|| unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let lines = inspect_inner().unwrap_or_else(|e| vec![format!("error: {e}")]);
        if init.is_ok() {
            CoUninitialize();
        }
        lines
    })
    .join()
    .unwrap_or_default()
}

unsafe fn inspect_inner() -> windows::core::Result<Vec<String>> {
    let policy: IPolicyConfigVista =
        unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_VISTA, None, CLSCTX_ALL)? };
    let mut lines = Vec::new();
    for endpoint in unsafe { find_endpoints()? } {
        let flow = if endpoint.flow == eRender {
            "render"
        } else {
            "capture"
        };
        let read = |key: &PROPERTYKEY| {
            let mut value = PROPVARIANT::default();
            unsafe { policy.GetPropertyValue(endpoint.pcwstr(), false.into(), key, &mut value) }
                .ok()
                .ok()
                .map(|()| value.to_string())
                .unwrap_or_else(|| "(unset)".into())
        };
        lines.push(format!(
            "{flow}: format {:?}, disable_sysfx {}, physical_speakers {}, driver {:?}",
            unsafe { current_format(&policy, &endpoint) },
            read(&PKEY_DISABLE_SYSFX),
            read(&PKEY_PHYSICAL_SPEAKERS),
            endpoint
                .driver_info
                .as_deref()
                .and_then(|i| i.split(':').nth(3)),
        ));
        let candidates: &[Format] = if endpoint.flow == eRender {
            &[HEADPHONE_FORMAT]
        } else {
            &[
                Format {
                    channels: 1,
                    rate: 96_000,
                    bits: 24,
                },
                Format {
                    channels: 1,
                    rate: 48_000,
                    bits: 24,
                },
                Format {
                    channels: 1,
                    rate: 48_000,
                    bits: 16,
                },
                Format {
                    channels: 1,
                    rate: 16_000,
                    bits: 16,
                },
            ]
        };
        let mask = if endpoint.flow == eRender {
            SPEAKER_STEREO
        } else {
            0x4
        };
        for f in candidates {
            let ok = unsafe { supported(&endpoint, &WaveFormatExtensible::pcm(*f, mask)) };
            lines.push(format!(
                "  exclusive {f:?}: {}",
                if ok { "supported" } else { "no" }
            ));
        }
    }
    Ok(lines)
}

unsafe fn apply_defaults_inner(apply: bool) -> Summary {
    let mut summary = Summary::default();
    let endpoints = match unsafe { find_endpoints() } {
        Ok(e) => e,
        Err(e) => {
            summary.problems.push(format!("audio endpoints: {e}"));
            return summary;
        }
    };
    let policy: IPolicyConfigVista =
        match unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_VISTA, None, CLSCTX_ALL) } {
            Ok(p) => p,
            Err(e) => {
                summary.problems.push(format!("policy config: {e}"));
                return summary;
            }
        };

    for endpoint in &endpoints {
        if summary.driver_version.is_none() {
            summary.driver_version = endpoint
                .driver_info
                .as_deref()
                .and_then(|info| info.split(':').nth(3))
                .map(str::to_string);
        }

        if !apply {
            let format = unsafe { current_format(&policy, endpoint) }.map(|(f, _)| f);
            if endpoint.flow == eRender {
                summary.headphones = format;
            } else {
                summary.microphone = format;
            }
            continue;
        }

        if let Err(e) = unsafe { set_u32(&policy, endpoint, &PKEY_DISABLE_SYSFX, 1) } {
            summary.problems.push(format!("enhancements off: {e}"));
        }

        let result = if endpoint.flow == eRender {
            if let Err(e) =
                unsafe { set_u32(&policy, endpoint, &PKEY_PHYSICAL_SPEAKERS, SPEAKER_STEREO) }
            {
                summary.problems.push(format!("stereo: {e}"));
            }
            unsafe { ensure_format(&policy, endpoint, &[HEADPHONE_FORMAT], SPEAKER_STEREO) }
                .map(|f| summary.headphones = Some(f))
        } else {
            unsafe { ensure_best_capture_format(&policy, endpoint) }
                .map(|f| summary.microphone = Some(f))
        };
        if let Err(e) = result {
            summary.problems.push(e);
        }
    }
    summary
}

unsafe fn find_endpoints() -> windows::core::Result<Vec<Endpoint>> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let collection = unsafe { enumerator.EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE)? };
    let mut endpoints = Vec::new();
    for i in 0..unsafe { collection.GetCount()? } {
        let device = unsafe { collection.Item(i)? };
        let store = unsafe { device.OpenPropertyStore(STGM_READ)? };
        let read = |k: &PROPERTYKEY| unsafe { store.GetValue(k) }.ok().map(|v| v.to_string());
        let devnode = read(&PKEY_ENDPOINT_DEVNODE).unwrap_or_default();
        let name = read(&PKEY_FRIENDLY_NAME).unwrap_or_default();
        if !devnode.to_ascii_uppercase().contains("VID_041E&PID_3130") && !name.contains("BT-W5") {
            continue;
        }
        let flow = unsafe { device.cast::<IMMEndpoint>()?.GetDataFlow()? };
        if flow != eRender && flow != eCapture {
            continue;
        }
        let raw_id = unsafe { device.GetId()? };
        let id: Vec<u16> = unsafe { raw_id.as_wide() }
            .iter()
            .copied()
            .chain(Some(0))
            .collect();
        unsafe { CoTaskMemFree(Some(raw_id.0 as *const c_void)) };
        endpoints.push(Endpoint {
            id,
            device,
            flow,
            driver_info: read(&PKEY_DRIVER_INFO),
        });
    }
    Ok(endpoints)
}

unsafe fn set_u32(
    policy: &IPolicyConfigVista,
    endpoint: &Endpoint,
    key: &PROPERTYKEY,
    value: u32,
) -> windows::core::Result<()> {
    let mut current = PROPVARIANT::default();
    let read =
        unsafe { policy.GetPropertyValue(endpoint.pcwstr(), false.into(), key, &mut current) };
    if read.is_ok() && u32::try_from(&current).ok() == Some(value) {
        return Ok(());
    }
    let new = propvariant_u32(value);
    unsafe { policy.SetPropertyValue(endpoint.pcwstr(), false.into(), key, &new) }.ok()
}

unsafe fn current_format(
    policy: &IPolicyConfigVista,
    endpoint: &Endpoint,
) -> Option<(Format, u32)> {
    let mut ptr: *mut WAVEFORMATEX = std::ptr::null_mut();
    unsafe { policy.GetDeviceFormat(endpoint.pcwstr(), false.into(), &mut ptr) }
        .ok()
        .ok()?;
    if ptr.is_null() {
        return None;
    }
    let base = unsafe { std::ptr::read_unaligned(ptr) };
    let result = match (base.wFormatTag, base.cbSize) {
        (WAVE_FORMAT_EXTENSIBLE, size) if size >= 22 => {
            let ext = unsafe { std::ptr::read_unaligned(ptr.cast::<WaveFormatExtensible>()) };
            let (channels, rate, valid_bits, mask, sub) = (
                ext.channels,
                ext.rate,
                ext.valid_bits,
                ext.channel_mask,
                ext.sub_format,
            );
            (sub == KSDATAFORMAT_SUBTYPE_PCM).then_some((
                Format {
                    channels,
                    rate,
                    bits: valid_bits,
                },
                mask,
            ))
        }
        (WAVE_FORMAT_PCM, _) => Some((
            Format {
                channels: base.nChannels,
                rate: base.nSamplesPerSec,
                bits: base.wBitsPerSample,
            },
            0,
        )),
        _ => None,
    };
    unsafe { CoTaskMemFree(Some(ptr as *const c_void)) };
    result
}

unsafe fn supported(endpoint: &Endpoint, format: &WaveFormatExtensible) -> bool {
    let Ok(client) = (unsafe { endpoint.device.Activate::<IAudioClient>(CLSCTX_ALL, None) }) else {
        return false;
    };
    unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, format.as_wave(), None) }.is_ok()
}

/// Sets the first supported format of `candidates` unless the endpoint already uses one of
/// them. Returns the format in effect afterwards.
unsafe fn ensure_format(
    policy: &IPolicyConfigVista,
    endpoint: &Endpoint,
    candidates: &[Format],
    channel_mask: u32,
) -> Result<Format, String> {
    let current = unsafe { current_format(policy, endpoint) };
    let wanted = candidates
        .iter()
        .copied()
        .find(|f| unsafe { supported(endpoint, &WaveFormatExtensible::pcm(*f, channel_mask)) });
    let Some(wanted) = wanted else {
        return current
            .map(|(f, _)| f)
            .ok_or_else(|| "no supported format found".to_string());
    };
    if current.map(|(f, _)| f) == Some(wanted) {
        return Ok(wanted);
    }
    let wave = WaveFormatExtensible::pcm(wanted, channel_mask);
    unsafe { policy.SetDeviceFormat(endpoint.pcwstr(), wave.as_wave(), std::ptr::null()) }
        .ok()
        .map_err(|e| format!("set format: {e}"))?;
    Ok(wanted)
}

unsafe fn ensure_best_capture_format(
    policy: &IPolicyConfigVista,
    endpoint: &Endpoint,
) -> Result<Format, String> {
    let (current, mask) = unsafe { current_format(policy, endpoint) }.unwrap_or((
        Format {
            channels: 1,
            rate: 48_000,
            bits: 16,
        },
        0,
    ));
    let channels = current.channels.max(1);
    let mask = if mask != 0 {
        mask
    } else if channels == 1 {
        0x4
    } else {
        SPEAKER_STEREO
    };
    let mut candidates = Vec::new();
    for rate in [192_000, 96_000, 48_000, 44_100, 32_000, 16_000] {
        for bits in [32, 24, 16] {
            candidates.push(Format {
                channels,
                rate,
                bits,
            });
        }
    }
    unsafe { ensure_format(policy, endpoint, &candidates, mask) }
}
