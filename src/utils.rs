/// If the environment variable `WGPU_ADAPTER_NAME` is set, this function will attempt to
/// initialize the adapter with that name. If it is not set, it will attempt to initialize
/// the adapter which supports the required features.
pub(crate) async fn get_adapter_with_capabilities_or_from_env(
    instance: &wgpu::Instance,
    required_features: &wgpu::Features,
    required_downlevel_capabilities: &wgpu::DownlevelCapabilities,
    surface: &Option<&wgpu::Surface<'_>>,
) -> wgpu::Adapter {
    use wgpu::Backends;
    if std::env::var("WGPU_ADAPTER_NAME").is_ok() {
        let adapter = wgpu::util::initialize_adapter_from_env_or_default(instance, *surface)
            .await
            .expect("No suitable GPU adapters found on the system!");

        let adapter_info = adapter.get_info();
        log::info!("Using {} ({:?})", adapter_info.name, adapter_info.backend);

        let adapter_features = adapter.features();
        assert!(
            adapter_features.contains(*required_features),
            "Adapter does not support required features for this app: {:?}",
            *required_features - adapter_features
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
            required_downlevel_capabilities.flags - downlevel_capabilities.flags
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
            if !adapter_features.contains(required_features) {
                continue;
            } else {
                chosen_adapter = Some(adapter);
                break;
            }
        }

        chosen_adapter.expect("No suitable GPU adapters found on the system!")
    }
}
