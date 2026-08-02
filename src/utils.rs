/// Fails fast with an error message. `panic!`/`unwrap`/`expect`/`exit` are all
/// denied lints in this crate, so a hard `abort` after logging is the sanctioned
/// failure path for invariants that cannot be handled gracefully.
pub fn fatal(msg: &str) -> ! {
    log::error!("{msg}");
    std::process::abort();
}

// ---------------------------------------------------------------------------
// Cast helpers.
//
// These conversions are either lossy by nature (no `From`/`TryFrom` impl
// exists) or only lossless within the value ranges this crate actually uses
// (window sizes, chunk counts, GPU counters). Each helper confines the
// `as`-cast (and the necessarily-scoped allow) to one documented place so the
// rest of the crate can stay at `as_conversions = deny`.
// ---------------------------------------------------------------------------

/// `u32` -> `f32`. Exact while `v <= 2^24`; every value this crate converts
/// (texture dims, window sizes, binding counts) is far below that.
#[allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
pub const fn u32_to_f32(v: u32) -> f32 {
    v as f32
}

/// `i32` -> `f32`. Exact while `|v| <= 2^24`; used only for small world/offset
/// coordinates.
#[allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
pub const fn i32_to_f32(v: i32) -> f32 {
    v as f32
}

/// `usize` -> `f32`. Exact while `v <= 2^24`; used only for small counts.
#[allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]
pub const fn usize_to_f32(v: usize) -> f32 {
    v as f32
}

/// `u64` -> `f32`. Used only for elapsed-seconds in log output.
#[allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]
pub const fn u64_to_f32(v: u64) -> f32 {
    v as f32
}

/// `u64` -> `f64`. Exact while `v <= 2^53`; used only for GPU profiling
/// counters that cannot realistically exceed that (2^53 fragments/frame).
#[allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]
pub const fn u64_to_f64(v: u64) -> f64 {
    v as f64
}

/// `f32` -> `i32`, truncating toward zero (the semantics of `as i32`).
/// Callers that need rounding should apply `.round()` first. Saturates for
/// out-of-range inputs.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub const fn f32_to_i32(v: f32) -> i32 {
    v as i32
}

/// `f32` -> `i16`, truncating toward zero (the semantics of `as i16`).
/// Callers that need rounding should apply `.round()` first. Saturates for
/// out-of-range inputs.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub const fn f32_to_i16(v: f32) -> i16 {
    v as i16
}

/// `f64` -> `f32`. Used only for mouse-look deltas (sub-degree values).
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
pub const fn f64_to_f32(v: f64) -> f32 {
    v as f32
}

/// If the environment variable `WGPU_ADAPTER_NAME` is set, this function will attempt to
/// initialize the adapter with that name. If it is not set, it will attempt to initialize
/// the adapter which supports the required features.
pub async fn get_adapter_with_capabilities_or_from_env(
    instance: &wgpu::Instance,
    required_features: &wgpu::Features,
    required_downlevel_capabilities: &wgpu::DownlevelCapabilities,
    surface: Option<&wgpu::Surface<'_>>,
) -> wgpu::Adapter {
    use wgpu::Backends;
    if std::env::var("WGPU_ADAPTER_NAME").is_ok() {
        let adapter = wgpu::util::initialize_adapter_from_env_or_default(instance, surface)
            .await
            .unwrap_or_else(|e| {
                fatal(&format!(
                    "No suitable GPU adapters found on the system! ({e})"
                ))
            });

        let adapter_info = adapter.get_info();
        log::info!("Using {} ({:?})", adapter_info.name, adapter_info.backend);

        let adapter_features = adapter.features();
        assert!(
            adapter_features.contains(*required_features),
            "Adapter does not support required features for this app: {:?}",
            required_features.difference(adapter_features)
        );

        let downlevel_capabilities = adapter.get_downlevel_capabilities();
        assert!(
            downlevel_capabilities.shader_model >= required_downlevel_capabilities.shader_model,
            "Adapter does not support the minimum shader model required to run this app: {:?}",
            required_downlevel_capabilities.shader_model
        );
        assert!(
            downlevel_capabilities
                .flags
                .contains(required_downlevel_capabilities.flags),
            "Adapter does not support the downlevel capabilities required to run this app: {:?}",
            required_downlevel_capabilities
                .flags
                .difference(downlevel_capabilities.flags)
        );
        adapter
    } else {
        let adapters = instance.enumerate_adapters(Backends::all()).await;

        let mut chosen_adapter = None;
        for adapter in adapters {
            if let Some(surface) = surface
                && !adapter.is_surface_supported(surface)
            {
                continue;
            }

            let required_features = *required_features;
            let adapter_features = adapter.features();
            if adapter_features.contains(required_features) {
                chosen_adapter = Some(adapter);
                break;
            }
        }

        chosen_adapter.unwrap_or_else(|| fatal("No suitable GPU adapters found on the system!"))
    }
}
